//! Tests for micromegas_operator::reconcile::screen.

use k8s_openapi::api::core::v1::ConfigMap;
use kube::core::ObjectMeta;
use micromegas_operator::conditions::reasons;
use micromegas_operator::crds::{Screen, ScreenInstanceStatus, ScreenSpec};
use micromegas_operator::reconcile::screen::{
    InstanceOutcome, aggregate, config_from_configmap, desired_identity,
};
use serde_json::json;

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
