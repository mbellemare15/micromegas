use super::instance::{build_client, instance_is_ready, matching_instances, resync_interval};
use super::{Context, Error, patch_status};
use crate::client::ApiError;
use crate::conditions::{self, reasons};
use crate::crds::{MicromegasInstance, Screen, ScreenInstanceStatus};
use crate::plan::{self, Action as PlanAction, Desired};
use analytics_web_api::{ScreenType, validate_folder_path, validate_name};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::runtime::controller::Action;
use kube::runtime::events::{Event, EventType};
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Api, Resource, ResourceExt};
use micromegas::tracing::prelude::*;
use std::sync::Arc;
use std::time::Duration;

pub const FINALIZER: &str = "micromegas.info/screen";
const DEFAULT_REQUEUE: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct Failure {
    pub reason: &'static str,
    pub message: String,
}

#[derive(Debug)]
pub struct Identity {
    pub name: String,
    pub screen_type: String,
    pub folder_path: String,
    pub managed_by: String,
}

pub enum InstanceOutcome {
    Synced {
        status: ScreenInstanceStatus,
    },
    Failed {
        status: ScreenInstanceStatus,
        reason: &'static str,
        transient: bool,
    },
}

pub fn desired_identity(screen: &Screen, cluster: &str) -> Result<Identity, Failure> {
    let cr_name = screen.name_any();
    let namespace = screen.namespace().unwrap_or_default();
    let name = screen.spec.name.clone().unwrap_or_else(|| cr_name.clone());
    validate_name(&name).map_err(|e| Failure {
        reason: reasons::INVALID_NAME,
        message: format!("screen name '{name}': {}", e.message),
    })?;
    validate_folder_path(&screen.spec.folder_path).map_err(|e| Failure {
        reason: reasons::INVALID_NAME,
        message: format!("folderPath '{}': {}", screen.spec.folder_path, e.message),
    })?;
    screen
        .spec
        .screen_type
        .parse::<ScreenType>()
        .map_err(|e| Failure {
            reason: reasons::INVALID_CONFIG,
            message: e.to_string(),
        })?;
    Ok(Identity {
        name,
        screen_type: screen.spec.screen_type.clone(),
        folder_path: screen.spec.folder_path.clone(),
        managed_by: plan::managed_by(cluster, &namespace, &cr_name),
    })
}

pub fn config_from_configmap(
    cm: Option<&ConfigMap>,
    cm_name: &str,
    key: &str,
) -> Result<serde_json::Value, Failure> {
    let cm = cm.ok_or_else(|| Failure {
        reason: reasons::CONFIG_MAP_NOT_FOUND,
        message: format!("configmap '{cm_name}' not found"),
    })?;
    let raw = cm
        .data
        .as_ref()
        .and_then(|d| d.get(key))
        .ok_or_else(|| Failure {
            reason: reasons::CONFIG_MAP_NOT_FOUND,
            message: format!("configmap '{cm_name}' has no key '{key}'"),
        })?;
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| Failure {
        reason: reasons::INVALID_CONFIG,
        message: format!("configmap '{cm_name}' key '{key}' is not valid JSON: {e}"),
    })?;
    if !value.is_object() {
        return Err(Failure {
            reason: reasons::INVALID_CONFIG,
            message: format!("configmap '{cm_name}' key '{key}' must hold a JSON object"),
        });
    }
    Ok(value)
}

pub fn aggregate(outcomes: &[InstanceOutcome]) -> (bool, &'static str, String) {
    if outcomes.is_empty() {
        return (
            false,
            reasons::NO_MATCHING_INSTANCE,
            "no MicromegasInstance matches instanceSelector".into(),
        );
    }
    for outcome in outcomes {
        if let InstanceOutcome::Failed { status, reason, .. } = outcome {
            let detail = status.error.clone().unwrap_or_default();
            return (
                false,
                reason,
                format!("instance {}/{}: {detail}", status.namespace, status.name),
            );
        }
    }
    (
        true,
        reasons::SYNCED,
        format!("synced to {} instance(s)", outcomes.len()),
    )
}

/// The per-instance status the previous reconcile recorded, or an empty list
/// for a Screen that has never synced. Used both to carry status forward
/// through a transient error and to look up one instance's prior entry.
pub fn prior_instances(screen: &Screen) -> Vec<ScreenInstanceStatus> {
    screen
        .status
        .as_ref()
        .map(|s| s.instances.clone())
        .unwrap_or_default()
}

/// Looks up the status the previous reconcile recorded for one instance, so a
/// fresh sync can carry its config_hash/last_synced_at forward instead of
/// wiping them (see `sync_success_status`).
pub fn prior_instance_status<'a>(
    previous: &'a [ScreenInstanceStatus],
    namespace: &str,
    name: &str,
) -> Option<&'a ScreenInstanceStatus> {
    previous
        .iter()
        .find(|s| s.namespace == namespace && s.name == name)
}

/// Builds the per-instance status after a successful sync. A `NoOp` plan
/// action must leave config_hash/last_synced_at exactly as they were: writing
/// a fresh timestamp on every reconcile turns steady state into a self
/// sustaining loop (status patch -> resourceVersion bump -> watch fires ->
/// reconcile -> fresh timestamp -> patch again), which also makes
/// resyncInterval meaningless. Only Create/Update advance them.
pub fn sync_success_status(
    mut status: ScreenInstanceStatus,
    changed: bool,
    desired: &Desired,
    now: &str,
) -> ScreenInstanceStatus {
    if changed {
        status.config_hash = Some(plan::config_hash(&desired.config));
        status.last_synced_at = Some(now.to_string());
    }
    status
}

/// `matching_instances` returns instances in the reflector store's (arbitrary)
/// hash order; sorting keeps `status.instances` and aggregate's "first
/// failure" deterministic across reconciles.
pub fn sorted_by_namespace_name(
    mut instances: Vec<Arc<MicromegasInstance>>,
) -> Vec<Arc<MicromegasInstance>> {
    instances.sort_by(|a, b| {
        let key_a = (a.namespace().unwrap_or_default(), a.name_any());
        let key_b = (b.namespace().unwrap_or_default(), b.name_any());
        key_a.cmp(&key_b)
    });
    instances
}

enum ResolveError {
    Terminal(Failure),
    Transient(String),
}

async fn resolve_config(screen: &Screen, ctx: &Context) -> Result<serde_json::Value, ResolveError> {
    if let Some(config) = &screen.spec.config {
        return Ok(config.clone());
    }
    let Some(from) = &screen.spec.config_from else {
        return Err(ResolveError::Terminal(Failure {
            reason: reasons::INVALID_CONFIG,
            message: "neither config nor configFrom is set".into(),
        }));
    };
    let reference = &from.config_map_key_ref;
    let api: Api<ConfigMap> =
        Api::namespaced(ctx.client.clone(), &screen.namespace().unwrap_or_default());
    let cm = match api.get(&reference.name).await {
        Ok(cm) => Some(cm),
        Err(kube::Error::Api(e)) if e.code == 404 => None,
        // A transient Kubernetes API error reading the ConfigMap is not proof
        // the reference is wrong; treat it as retryable rather than failing
        // the Screen outright.
        Err(e) => return Err(ResolveError::Transient(e.to_string())),
    };
    config_from_configmap(cm.as_ref(), &reference.name, &reference.key)
        .map_err(ResolveError::Terminal)
}

fn instance_status(
    instance: &MicromegasInstance,
    screen_name: &str,
    prior: Option<&ScreenInstanceStatus>,
) -> ScreenInstanceStatus {
    ScreenInstanceStatus {
        name: instance.name_any(),
        namespace: instance.namespace().unwrap_or_default(),
        screen_name: screen_name.to_string(),
        config_hash: prior.and_then(|p| p.config_hash.clone()),
        last_synced_at: prior.and_then(|p| p.last_synced_at.clone()),
        error: None,
    }
}

async fn sync_one(
    screen: &Screen,
    desired: &Desired,
    instance: &MicromegasInstance,
    prior: Option<&ScreenInstanceStatus>,
    ctx: &Context,
) -> InstanceOutcome {
    let status = instance_status(instance, &desired.name, prior);
    let fail = |mut status: ScreenInstanceStatus,
                reason: &'static str,
                message: String,
                transient: bool| {
        status.error = Some(message);
        InstanceOutcome::Failed {
            status,
            reason,
            transient,
        }
    };
    if !instance_is_ready(instance) {
        return fail(
            status,
            reasons::INSTANCE_NOT_READY,
            "instance is not Ready".into(),
            false,
        );
    }
    let client = match build_client(instance, ctx).await {
        Ok(c) => c,
        Err(e) => return fail(status, reasons::INSTANCE_NOT_READY, e.to_string(), true),
    };
    let current = match client.get_screen(&desired.name).await {
        Ok(c) => c,
        Err(e) => return fail(status, reasons::API_ERROR, e.to_string(), e.is_transient()),
    };
    let action = plan::plan(desired, current.as_ref());
    let result = match &action {
        PlanAction::Create(req) => client.create_screen(req).await.map(|_| "Created"),
        PlanAction::Update(req) => client
            .update_screen(&desired.name, req)
            .await
            .map(|_| "Updated"),
        PlanAction::NoOp => Ok("InSync"),
        PlanAction::Conflict(message) => {
            publish(ctx, screen, EventType::Warning, "Conflict", message).await;
            return fail(status, reasons::CONFLICT, message.clone(), false);
        }
    };
    match result {
        Ok(verb) => {
            let changed = verb != "InSync";
            if changed {
                publish(
                    ctx,
                    screen,
                    EventType::Normal,
                    verb,
                    &format!(
                        "{verb} screen '{}' on {}",
                        desired.name,
                        instance.name_any()
                    ),
                )
                .await;
            }
            let now = chrono::Utc::now().to_rfc3339();
            InstanceOutcome::Synced {
                status: sync_success_status(status, changed, desired, &now),
            }
        }
        Err(e @ ApiError::BadRequest(_)) => {
            fail(status, reasons::INVALID_CONFIG, e.to_string(), false)
        }
        // Unauthorized covers both 401 and 403 (see client::classify); it means the
        // credentials or grants are wrong, not that the server is momentarily
        // unavailable, so it must not be treated as transient.
        Err(ApiError::Unauthorized) => fail(
            status,
            reasons::API_ERROR,
            "server rejected the request as unauthorized (401/403); check the instance credentials"
                .into(),
            false,
        ),
        Err(e) => fail(status, reasons::API_ERROR, e.to_string(), e.is_transient()),
    }
}

async fn publish(ctx: &Context, screen: &Screen, type_: EventType, action: &str, note: &str) {
    let event = Event {
        type_,
        reason: action.to_string(),
        note: Some(note.to_string()),
        action: action.to_string(),
        secondary: None,
    };
    if let Err(e) = ctx.recorder.publish(&event, &screen.object_ref(&())).await {
        warn!("event publish failed: {e}");
    }
}

async fn write_status(
    api: &Api<Screen>,
    screen: &Screen,
    ok: bool,
    reason: &str,
    message: &str,
    instances: Vec<ScreenInstanceStatus>,
) -> Result<(), Error> {
    let generation = screen.metadata.generation;
    let mut status = screen.status.clone().unwrap_or_default();
    status.observed_generation = generation;
    status.instances = instances;
    conditions::upsert(
        &mut status.conditions,
        conditions::ready(ok, reason, message, generation),
    );
    patch_status(api, &screen.name_any(), &status).await?;
    Ok(())
}

async fn apply(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let namespace = screen.namespace().unwrap_or_default();
    let api: Api<Screen> = Api::namespaced(ctx.client.clone(), &namespace);
    let key = format!("{namespace}/{}", screen.name_any());

    let identity = match desired_identity(&screen, &ctx.cluster_name) {
        Ok(id) => id,
        Err(f) => {
            write_status(&api, &screen, false, f.reason, &f.message, vec![]).await?;
            return Ok(Action::await_change());
        }
    };
    let config = match resolve_config(&screen, &ctx).await {
        Ok(c) => c,
        Err(ResolveError::Terminal(f)) => {
            write_status(&api, &screen, false, f.reason, &f.message, vec![]).await?;
            return Ok(Action::await_change());
        }
        Err(ResolveError::Transient(message)) => {
            // A ConfigMap read hiccup is not evidence that the last known sync
            // state is wrong; keep the carried-forward per-instance status so a
            // retry doesn't blank config_hash/last_synced_at for every instance.
            write_status(
                &api,
                &screen,
                false,
                reasons::API_ERROR,
                &message,
                prior_instances(&screen),
            )
            .await?;
            return Ok(Action::requeue(ctx.backoff.next(&key)));
        }
    };
    let desired = Desired {
        name: identity.name,
        screen_type: identity.screen_type,
        folder_path: identity.folder_path,
        config,
        managed_by: identity.managed_by,
    };

    let instances = sorted_by_namespace_name(
        matching_instances(&screen.spec.instance_selector, ctx.instances.state())
            .map_err(Error::Transient)?,
    );
    let previous = prior_instances(&screen);
    let mut outcomes = Vec::with_capacity(instances.len());
    let mut requeue = DEFAULT_REQUEUE;
    for instance in &instances {
        if let Ok(interval) = resync_interval(&instance.spec) {
            requeue = requeue.min(interval);
        }
        let inst_namespace = instance.namespace().unwrap_or_default();
        let inst_name = instance.name_any();
        let prior = prior_instance_status(&previous, &inst_namespace, &inst_name);
        outcomes.push(sync_one(&screen, &desired, instance, prior, &ctx).await);
    }

    let (ok, reason, message) = aggregate(&outcomes);
    let statuses = outcomes
        .iter()
        .map(|o| match o {
            InstanceOutcome::Synced { status } | InstanceOutcome::Failed { status, .. } => {
                status.clone()
            }
        })
        .collect();
    write_status(&api, &screen, ok, reason, &message, statuses).await?;

    let transient = outcomes.iter().any(|o| {
        matches!(
            o,
            InstanceOutcome::Failed {
                transient: true,
                ..
            }
        )
    });
    if transient {
        return Ok(Action::requeue(ctx.backoff.next(&key)));
    }
    ctx.backoff.reset(&key);
    if instances.is_empty() {
        return Ok(Action::await_change());
    }
    Ok(Action::requeue(requeue))
}

async fn cleanup(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let identity = match desired_identity(&screen, &ctx.cluster_name) {
        Ok(id) => id,
        // Nothing valid could ever have been written under an invalid identity.
        Err(_) => return Ok(Action::await_change()),
    };
    let instances = sorted_by_namespace_name(
        matching_instances(&screen.spec.instance_selector, ctx.instances.state())
            .map_err(Error::Transient)?,
    );
    for instance in instances {
        let client = build_client(&instance, &ctx)
            .await
            .map_err(|e| Error::Transient(e.to_string()))?;
        match client.get_screen(&identity.name).await {
            Ok(Some(current))
                if current.managed_by.as_deref() == Some(identity.managed_by.as_str()) =>
            {
                client
                    .delete_screen(&identity.name)
                    .await
                    .map_err(|e| Error::Transient(e.to_string()))?;
                info!(
                    "deleted screen '{}' from {}",
                    identity.name,
                    instance.name_any()
                );
                publish(
                    &ctx,
                    &screen,
                    EventType::Normal,
                    "Deleted",
                    &format!(
                        "deleted screen '{}' on {}",
                        identity.name,
                        instance.name_any()
                    ),
                )
                .await;
            }
            Ok(_) => {}
            Err(e) => return Err(Error::Transient(e.to_string())),
        }
    }
    Ok(Action::await_change())
}

pub async fn reconcile(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let namespace = screen.namespace().unwrap_or_default();
    let api: Api<Screen> = Api::namespaced(ctx.client.clone(), &namespace);
    let ctx2 = ctx.clone();
    finalizer(&api, FINALIZER, screen, |event| async move {
        match event {
            FinalizerEvent::Apply(s) => apply(s, ctx2).await,
            FinalizerEvent::Cleanup(s) => cleanup(s, ctx2).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

pub fn error_policy(screen: Arc<Screen>, err: &Error, ctx: Arc<Context>) -> Action {
    let key = format!(
        "{}/{}",
        screen.namespace().unwrap_or_default(),
        screen.name_any()
    );
    warn!("screen {key} reconcile failed: {err}");
    Action::requeue(ctx.backoff.next(&key))
}
