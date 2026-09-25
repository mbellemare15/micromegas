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
