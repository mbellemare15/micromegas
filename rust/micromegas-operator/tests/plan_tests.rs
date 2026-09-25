//! Tests for micromegas_operator::plan.

use analytics_web_api::Screen;
use micromegas_operator::plan::{Action, Desired, config_hash, managed_by, plan};
use serde_json::{Value, json};

fn desired() -> Desired {
    Desired {
        name: "overview".into(),
        screen_type: "notebook".into(),
        folder_path: "team/prod".into(),
        config: json!({"timeRangeFrom": "now-1h", "cells": [{"id": 1, "type": "table"}]}),
        managed_by: managed_by("prod-eu", "game-system", "overview"),
    }
}

fn current(managed_by: Option<&str>, config: Value, folder: &str) -> Screen {
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
