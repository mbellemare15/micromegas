//! Tests for micromegas_operator::reconcile::screen.

use k8s_openapi::api::core::v1::ConfigMap;
use kube::core::ObjectMeta;
use micromegas_operator::conditions::reasons;
use micromegas_operator::crds::{
    MicromegasInstance, MicromegasInstanceSpec, Screen, ScreenInstanceStatus, ScreenSpec,
    ScreenStatus,
};
use micromegas_operator::plan::{self, Desired};
use micromegas_operator::reconcile::screen::{
    InstanceOutcome, aggregate, config_from_configmap, desired_identity, prior_instance_status,
    prior_instances, sorted_by_namespace_name, sync_success_status,
};
use serde_json::json;
use std::sync::Arc;

fn screen(meta_name: &str, spec_name: Option<&str>, folder: &str) -> Screen {
    Screen {
        metadata: ObjectMeta {
            name: Some(meta_name.into()),
            namespace: Some("ns".into()),
            ..Default::default()
        },
        spec: ScreenSpec {
            instance_selector: Default::default(),
            name: spec_name.map(str::to_string),
            screen_type: "notebook".into(),
            folder_path: folder.into(),
            config: Some(json!({})),
            config_from: None,
        },
        status: None,
    }
}

#[test]
fn desired_identity_defaults_to_metadata_name() {
    let id = desired_identity(&screen("overview", None, "team/prod"), "c").unwrap();
    assert_eq!(id.name, "overview");
    assert_eq!(id.managed_by, "k8s://c/ns/overview");
    assert_eq!(id.folder_path, "team/prod");
}

#[test]
fn desired_identity_prefers_spec_name() {
    let id = desired_identity(&screen("cr-name", Some("server-name"), ""), "c").unwrap();
    assert_eq!(id.name, "server-name");
    assert_eq!(id.managed_by, "k8s://c/ns/cr-name");
}

#[test]
fn desired_identity_rejects_invalid_default_name() {
    for bad in ["my_screen", "ab", "new", "1abc"] {
        let err = desired_identity(&screen(bad, None, ""), "c").unwrap_err();
        assert_eq!(err.reason, reasons::INVALID_NAME, "{bad}");
    }
}

#[test]
fn desired_identity_rejects_invalid_folder_and_type() {
    let err = desired_identity(&screen("ok-name", None, "Bad/Path"), "c").unwrap_err();
    assert_eq!(err.reason, reasons::INVALID_NAME);
    let mut s = screen("ok-name", None, "");
    s.spec.screen_type = "widget".into();
    assert_eq!(
        desired_identity(&s, "c").unwrap_err().reason,
        reasons::INVALID_CONFIG
    );
}

fn configmap(data: &[(&str, &str)]) -> ConfigMap {
    ConfigMap {
        data: Some(
            data.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
        ..Default::default()
    }
}

#[test]
fn configmap_missing_is_not_found() {
    let err = config_from_configmap(None, "cm", "k").unwrap_err();
    assert_eq!(err.reason, reasons::CONFIG_MAP_NOT_FOUND);
    let err = config_from_configmap(Some(&configmap(&[("other", "{}")])), "cm", "k").unwrap_err();
    assert_eq!(err.reason, reasons::CONFIG_MAP_NOT_FOUND);
    assert!(err.message.contains("key 'k'"));
}

#[test]
fn configmap_value_must_be_json_object() {
    let err =
        config_from_configmap(Some(&configmap(&[("k", "{not json")])), "cm", "k").unwrap_err();
    assert_eq!(err.reason, reasons::INVALID_CONFIG);
    let err = config_from_configmap(Some(&configmap(&[("k", "[1,2]")])), "cm", "k").unwrap_err();
    assert_eq!(err.reason, reasons::INVALID_CONFIG);
    let ok =
        config_from_configmap(Some(&configmap(&[("k", r#"{"cells":[]}"#)])), "cm", "k").unwrap();
    assert_eq!(ok, json!({"cells": []}));
}

fn status(name: &str) -> ScreenInstanceStatus {
    ScreenInstanceStatus {
        name: name.into(),
        namespace: "m".into(),
        screen_name: "s".into(),
        config_hash: None,
        last_synced_at: None,
        error: None,
    }
}

#[test]
fn aggregate_all_synced_is_ready() {
    let (ok, reason, _) = aggregate(&[InstanceOutcome::Synced {
        status: status("a"),
    }]);
    assert!(ok);
    assert_eq!(reason, reasons::SYNCED);
}

#[test]
fn aggregate_reports_first_failure() {
    let outcomes = [
        InstanceOutcome::Synced {
            status: status("a"),
        },
        InstanceOutcome::Failed {
            status: status("b"),
            reason: reasons::CONFLICT,
            transient: false,
        },
        InstanceOutcome::Failed {
            status: status("c"),
            reason: reasons::API_ERROR,
            transient: true,
        },
    ];
    let (ok, reason, message) = aggregate(&outcomes);
    assert!(!ok);
    assert_eq!(reason, reasons::CONFLICT);
    assert!(message.contains("b"));
}

#[test]
fn aggregate_empty_is_no_matching_instance() {
    let (ok, reason, _) = aggregate(&[]);
    assert!(!ok);
    assert_eq!(reason, reasons::NO_MATCHING_INSTANCE);
}

fn instance_status(
    namespace: &str,
    name: &str,
    config_hash: &str,
    last_synced_at: &str,
) -> ScreenInstanceStatus {
    ScreenInstanceStatus {
        name: name.into(),
        namespace: namespace.into(),
        screen_name: "overview".into(),
        config_hash: Some(config_hash.into()),
        last_synced_at: Some(last_synced_at.into()),
        error: None,
    }
}

#[test]
fn prior_instance_status_matches_namespace_and_name() {
    let previous = [
        instance_status("m", "a", "sha256:aaa", "2024-01-01T00:00:00Z"),
        instance_status("m", "b", "sha256:bbb", "2024-01-02T00:00:00Z"),
    ];
    let found = prior_instance_status(&previous, "m", "b").expect("present");
    assert_eq!(found.config_hash.as_deref(), Some("sha256:bbb"));
    assert!(prior_instance_status(&previous, "m", "missing").is_none());
    assert!(prior_instance_status(&previous, "other-ns", "a").is_none());
}

#[test]
fn prior_instances_returns_status_instances_unchanged() {
    let mut screen = screen("overview", None, "");
    let carried = vec![
        instance_status("m", "a", "sha256:aaa", "2024-01-01T00:00:00Z"),
        instance_status("m", "b", "sha256:bbb", "2024-01-02T00:00:00Z"),
    ];
    screen.status = Some(ScreenStatus {
        instances: carried.clone(),
        ..Default::default()
    });

    assert_eq!(prior_instances(&screen), carried);
}

#[test]
fn prior_instances_empty_when_no_status() {
    let screen = screen("overview", None, "");
    assert!(screen.status.is_none());
    assert_eq!(prior_instances(&screen), Vec::new());
}

fn desired() -> Desired {
    Desired {
        name: "overview".into(),
        screen_type: "notebook".into(),
        folder_path: "".into(),
        config: json!({"cells": []}),
        managed_by: "k8s://c/ns/overview".into(),
    }
}

#[test]
fn sync_success_status_noop_keeps_prior_hash_and_time() {
    let prior = instance_status("m", "a", "sha256:old", "2024-01-01T00:00:00Z");
    let carried = ScreenInstanceStatus {
        error: None,
        ..prior.clone()
    };
    let result = sync_success_status(carried, false, &desired(), "2024-06-01T00:00:00Z");
    assert_eq!(result.config_hash, prior.config_hash);
    assert_eq!(result.last_synced_at, prior.last_synced_at);
}

#[test]
fn sync_success_status_change_sets_fresh_hash_and_time() {
    let prior = instance_status("m", "a", "sha256:old", "2024-01-01T00:00:00Z");
    let carried = ScreenInstanceStatus {
        error: None,
        ..prior.clone()
    };
    let result = sync_success_status(carried, true, &desired(), "2024-06-01T00:00:00Z");
    assert_eq!(
        result.config_hash,
        Some(plan::config_hash(&desired().config))
    );
    assert_eq!(
        result.last_synced_at,
        Some("2024-06-01T00:00:00Z".to_string())
    );
    assert_ne!(result.last_synced_at, prior.last_synced_at);
}

fn instance(namespace: &str, name: &str) -> Arc<MicromegasInstance> {
    Arc::new(MicromegasInstance {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some(namespace.into()),
            ..Default::default()
        },
        spec: MicromegasInstanceSpec {
            url: "http://x".into(),
            auth: None,
            resync_interval: "10m".into(),
        },
        status: None,
    })
}

#[test]
fn sorted_by_namespace_name_orders_deterministically() {
    let instances = vec![instance("z", "a"), instance("a", "b"), instance("a", "a")];
    let sorted = sorted_by_namespace_name(instances);
    let keys: Vec<(String, String)> = sorted
        .iter()
        .map(|i| {
            (
                i.metadata.namespace.clone().unwrap(),
                i.metadata.name.clone().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            ("a".to_string(), "a".to_string()),
            ("a".to_string(), "b".to_string()),
            ("z".to_string(), "a".to_string()),
        ]
    );
}
