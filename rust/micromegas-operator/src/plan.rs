//! Decides what to do for one screen on one instance. Pure so it can be tested
//! without a cluster or a server.

use analytics_web_api::{CreateScreenRequest, Screen, UpdateScreenRequest};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct Desired {
    pub name: String,
    pub screen_type: String,
    pub folder_path: String,
    pub config: Value,
    pub managed_by: String,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Create(CreateScreenRequest),
    Update(UpdateScreenRequest),
    NoOp,
    Conflict(String),
}

pub fn managed_by(cluster: &str, namespace: &str, name: &str) -> String {
    format!("k8s://{cluster}/{namespace}/{name}")
}

pub fn plan(desired: &Desired, current: Option<&Screen>) -> Action {
    let Some(current) = current else {
        return Action::Create(CreateScreenRequest {
            name: desired.name.clone(),
            screen_type: desired.screen_type.clone(),
            config: desired.config.clone(),
            managed_by: Some(desired.managed_by.clone()),
            folder_path: desired.folder_path.clone(),
        });
    };

    match current.managed_by.as_deref() {
        None => {
            return Action::Conflict(format!(
                "screen '{}' exists on the server and is not managed by the operator",
                current.name
            ));
        }
        Some(owner) if owner != desired.managed_by => {
            return Action::Conflict(format!("screen '{}' is managed by '{owner}'", current.name));
        }
        Some(_) => {}
    }

    if current.screen_type != desired.screen_type {
        return Action::Conflict(format!(
            "screen '{}' has type '{}' on the server but the spec says '{}'; screen type is immutable",
            current.name, current.screen_type, desired.screen_type
        ));
    }

    let config_changed = canonical(&current.config) != canonical(&desired.config);
    let folder_changed = current.folder_path != desired.folder_path;
    if !config_changed && !folder_changed {
        return Action::NoOp;
    }
    Action::Update(UpdateScreenRequest {
        config: config_changed.then(|| desired.config.clone()),
        managed_by: None,
        folder_path: folder_changed.then(|| desired.folder_path.clone()),
    })
}

/// Key-sorted rendering. serde_json's map ordering depends on the
/// `preserve_order` feature, which any crate in the workspace may enable, so
/// equality and hashing never rely on it.
pub fn canonical(value: &Value) -> String {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(k, v)| (k.clone(), sort(v)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(value).to_string()
}

pub fn config_hash(value: &Value) -> String {
    let digest = Sha256::digest(canonical(value).as_bytes());
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn desired() -> Desired {
        Desired {
            name: "overview".into(),
            screen_type: "notebook".into(),
            folder_path: "team/prod".into(),
            config: json!({"timeRangeFrom": "now-1h", "cells": [{"id": 1, "type": "table"}]}),
            managed_by: managed_by("prod-eu", "game-system", "overview"),
        }
    }

    fn current(managed_by: Option<&str>, config: serde_json::Value, folder: &str) -> Screen {
        Screen {
            name: "overview".into(),
            screen_type: "notebook".into(),
            config,
            created_by: Some("someone".into()),
            updated_by: None,
            created_at: None,
            updated_at: None,
            managed_by: managed_by.map(str::to_string),
            folder_path: folder.into(),
        }
    }

    #[test]
    fn managed_by_format() {
        assert_eq!(managed_by("c", "ns", "n"), "k8s://c/ns/n");
    }

    #[test]
    fn creates_when_absent() {
        let d = desired();
        match plan(&d, None) {
            Action::Create(req) => {
                assert_eq!(req.name, "overview");
                assert_eq!(req.screen_type, "notebook");
                assert_eq!(req.folder_path, "team/prod");
                assert_eq!(
                    req.managed_by.as_deref(),
                    Some("k8s://prod-eu/game-system/overview")
                );
                assert_eq!(req.config, d.config);
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn noop_when_equal_modulo_key_order() {
        let d = desired();
        let reordered = json!({"cells": [{"type": "table", "id": 1}], "timeRangeFrom": "now-1h"});
        let c = current(Some(&d.managed_by), reordered, "team/prod");
        assert_eq!(plan(&d, Some(&c)), Action::NoOp);
    }

    #[test]
    fn updates_config_only() {
        let d = desired();
        let c = current(Some(&d.managed_by), json!({"cells": []}), "team/prod");
        match plan(&d, Some(&c)) {
            Action::Update(req) => {
                assert_eq!(req.config, Some(d.config.clone()));
                assert_eq!(req.folder_path, None);
                assert_eq!(req.managed_by, None);
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn updates_folder_only() {
        let d = desired();
        let c = current(Some(&d.managed_by), d.config.clone(), "");
        match plan(&d, Some(&c)) {
            Action::Update(req) => {
                assert_eq!(req.config, None);
                assert_eq!(req.folder_path.as_deref(), Some("team/prod"));
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn conflict_when_unmanaged() {
        let d = desired();
        let c = current(None, d.config.clone(), "team/prod");
        assert!(matches!(plan(&d, Some(&c)), Action::Conflict(msg) if msg.contains("not managed")));
    }

    #[test]
    fn conflict_when_owned_by_other_cr_same_cluster() {
        let d = desired();
        let other = managed_by("prod-eu", "other-namespace", "overview");
        let c = current(Some(&other), d.config.clone(), "team/prod");
        assert!(matches!(plan(&d, Some(&c)), Action::Conflict(msg) if msg.contains(&other)));
    }

    #[test]
    fn conflict_when_owned_by_screens_cli() {
        let d = desired();
        let c = current(
            Some("https://github.com/org/dashboards.git"),
            d.config.clone(),
            "team/prod",
        );
        assert!(matches!(plan(&d, Some(&c)), Action::Conflict(_)));
    }

    #[test]
    fn conflict_when_screen_type_differs() {
        let d = desired();
        let mut c = current(Some(&d.managed_by), d.config.clone(), "team/prod");
        c.screen_type = "log".into();
        assert!(matches!(plan(&d, Some(&c)), Action::Conflict(msg) if msg.contains("immutable")));
    }

    #[test]
    fn hash_is_stable_across_key_order() {
        let a = json!({"a": 1, "b": {"c": [1, 2], "d": null}});
        let b = json!({"b": {"d": null, "c": [1, 2]}, "a": 1});
        assert_eq!(config_hash(&a), config_hash(&b));
        assert!(config_hash(&a).starts_with("sha256:"));
        assert_ne!(config_hash(&a), config_hash(&json!({"a": 2})));
    }
}
