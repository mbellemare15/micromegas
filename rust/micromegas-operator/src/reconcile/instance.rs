use super::{Context, Error, patch_status};
use crate::auth::{OidcClientCredentials, TokenCache};
use crate::client::{ApiError, Credentials, WebApiClient};
use crate::conditions::{self, reasons};
use crate::crds::{MicromegasInstance, MicromegasInstanceSpec};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::core::{Selector, SelectorExt};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use micromegas::tracing::prelude::*;
use std::sync::Arc;
use std::time::Duration;

const PROBE_INTERVAL: Duration = Duration::from_secs(300);
const MIN_RESYNC: Duration = Duration::from_secs(60);

pub fn resync_interval(spec: &MicromegasInstanceSpec) -> Result<Duration, String> {
    let parsed = humantime::parse_duration(&spec.resync_interval)
        .map_err(|e| format!("resyncInterval '{}' is invalid: {e}", spec.resync_interval))?;
    if parsed < MIN_RESYNC {
        return Err(format!(
            "resyncInterval '{}' must be at least 1m",
            spec.resync_interval
        ));
    }
    Ok(parsed)
}

pub fn instance_is_ready(instance: &MicromegasInstance) -> bool {
    instance
        .status
        .as_ref()
        .is_some_and(|s| conditions::is_ready(&s.conditions))
}

pub fn matching_instances(
    selector: &LabelSelector,
    candidates: Vec<Arc<MicromegasInstance>>,
) -> Result<Vec<Arc<MicromegasInstance>>, String> {
    let selector = Selector::try_from(selector.clone()).map_err(|e| e.to_string())?;
    if selector.selects_all() {
        return Ok(Vec::new());
    }
    Ok(candidates
        .into_iter()
        .filter(|inst| selector.matches(inst.labels()))
        .collect())
}

#[derive(Debug, thiserror::Error)]
pub enum BuildClientError {
    #[error("secret '{0}' not found or missing keys")]
    SecretNotFound(String),
    #[error("{0}")]
    InvalidSpec(String),
    #[error(transparent)]
    Kube(#[from] kube::Error),
}

pub async fn build_client(
    instance: &MicromegasInstance,
    ctx: &Context,
) -> Result<WebApiClient, BuildClientError> {
    let Some(auth) = &instance.spec.auth else {
        return Ok(WebApiClient::new(
            ctx.http.clone(),
            &instance.spec.url,
            Credentials::None,
        ));
    };
    let oidc = &auth.oidc_client_credentials;
    let namespace = instance
        .namespace()
        .ok_or_else(|| BuildClientError::InvalidSpec("instance has no namespace".into()))?;
    let secrets: Api<Secret> = Api::namespaced(ctx.client.clone(), &namespace);
    let secret = match secrets.get(&oidc.secret_ref.name).await {
        Ok(s) => s,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            return Err(BuildClientError::SecretNotFound(
                oidc.secret_ref.name.clone(),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    let read = |key: &str| -> Result<String, BuildClientError> {
        secret
            .data
            .as_ref()
            .and_then(|d| d.get(key))
            .and_then(|bytes| String::from_utf8(bytes.0.clone()).ok())
            .ok_or_else(|| {
                BuildClientError::SecretNotFound(format!("{}/{key}", oidc.secret_ref.name))
            })
    };
    let creds = OidcClientCredentials {
        issuer: oidc.issuer.clone(),
        client_id: read(&oidc.secret_ref.client_id_key)?,
        client_secret: read(&oidc.secret_ref.client_secret_key)?,
        audience: oidc.audience.clone(),
    };
    let fingerprint = creds.fingerprint();
    let uid = instance.uid().unwrap_or_default();
    let cache = {
        let mut tokens = ctx.tokens.lock().expect("token map mutex");
        match tokens.get(&uid) {
            Some((fp, cache)) if *fp == fingerprint => cache.clone(),
            _ => {
                let cache = Arc::new(TokenCache::new(ctx.http.clone(), creds));
                tokens.insert(uid, (fingerprint, cache.clone()));
                cache
            }
        }
    };
    Ok(WebApiClient::new(
        ctx.http.clone(),
        &instance.spec.url,
        Credentials::Oidc(cache),
    ))
}

async fn check(instance: &MicromegasInstance, ctx: &Context) -> Result<(), (&'static str, String)> {
    resync_interval(&instance.spec).map_err(|m| (reasons::INVALID_SPEC, m))?;
    let client = build_client(instance, ctx).await.map_err(|e| match e {
        BuildClientError::SecretNotFound(_) => (reasons::SECRET_NOT_FOUND, e.to_string()),
        BuildClientError::InvalidSpec(_) => (reasons::INVALID_SPEC, e.to_string()),
        BuildClientError::Kube(_) => (reasons::UNREACHABLE, e.to_string()),
    })?;
    client.probe().await.map_err(|e| match e {
        ApiError::Unauthorized => (reasons::UNAUTHORIZED, e.to_string()),
        ApiError::Token(_) => (reasons::TOKEN_ERROR, e.to_string()),
        other => (reasons::UNREACHABLE, other.to_string()),
    })
}

pub async fn reconcile(
    instance: Arc<MicromegasInstance>,
    ctx: Arc<Context>,
) -> Result<Action, Error> {
    let namespace = instance.namespace().unwrap_or_default();
    let name = instance.name_any();
    let api: Api<MicromegasInstance> = Api::namespaced(ctx.client.clone(), &namespace);

    let outcome = check(&instance, &ctx).await;
    let generation = instance.metadata.generation;
    let condition = match &outcome {
        Ok(()) => conditions::ready(true, reasons::CONNECTED, "probe succeeded", generation),
        Err((reason, message)) => {
            warn!("instance {namespace}/{name} not ready: {reason}: {message}");
            conditions::ready(false, reason, message, generation)
        }
    };
    let mut status = instance.status.clone().unwrap_or_default();
    status.observed_generation = generation;
    conditions::upsert(&mut status.conditions, condition);
    patch_status(&api, &name, &status).await?;

    let key = format!("{namespace}/{name}");
    Ok(match outcome {
        Ok(()) => {
            ctx.backoff.reset(&key);
            Action::requeue(PROBE_INTERVAL)
        }
        Err(_) => Action::requeue(ctx.backoff.next(&key)),
    })
}

pub fn error_policy(instance: Arc<MicromegasInstance>, err: &Error, ctx: Arc<Context>) -> Action {
    let key = format!(
        "{}/{}",
        instance.namespace().unwrap_or_default(),
        instance.name_any()
    );
    warn!("instance {key} reconcile failed: {err}");
    Action::requeue(ctx.backoff.next(&key))
}
