//! Tests for micromegas_operator::reconcile.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::core::ObjectMeta;
use micromegas_operator::crds::{
    MicromegasInstance, MicromegasInstanceSpec, MicromegasInstanceStatus,
};
use micromegas_operator::reconcile::Backoff;
use micromegas_operator::reconcile::instance::{
    instance_is_ready, matching_instances, resync_interval,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

// Tests from src/reconcile/instance.rs

fn spec(resync: &str) -> MicromegasInstanceSpec {
    MicromegasInstanceSpec {
        url: "http://x".into(),
        auth: None,
        resync_interval: resync.into(),
    }
}

#[test]
fn resync_interval_parses_humantime() {
    assert_eq!(
        resync_interval(&spec("10m")).unwrap(),
        Duration::from_secs(600)
    );
    assert_eq!(
        resync_interval(&spec("1h 30m")).unwrap(),
        Duration::from_secs(5400)
    );
}

#[test]
fn resync_interval_enforces_minimum() {
    assert!(
        resync_interval(&spec("30s"))
            .unwrap_err()
            .contains("at least 1m")
    );
    assert!(resync_interval(&spec("1m")).is_ok());
}

#[test]
fn resync_interval_rejects_garbage() {
    assert!(resync_interval(&spec("soon")).is_err());
}

fn instance(name: &str, labels: &[(&str, &str)]) -> Arc<MicromegasInstance> {
    Arc::new(MicromegasInstance {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some("micromegas".into()),
            labels: Some(
                labels
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<BTreeMap<_, _>>(),
            ),
            ..Default::default()
        },
        spec: spec("10m"),
        status: None,
    })
}

#[test]
fn matching_instances_filters_by_selector() {
    let selector = LabelSelector {
        match_labels: Some(
            [("env".to_string(), "prod".to_string())]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    };
    let all = vec![
        instance("a", &[("env", "prod")]),
        instance("b", &[("env", "dev")]),
        instance("c", &[]),
    ];
    let matched = matching_instances(&selector, all).unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].metadata.name.as_deref(), Some("a"));
}

#[test]
fn empty_selector_matches_nothing() {
    // An empty selector would otherwise select every instance; Screens must opt in explicitly.
    let all = vec![instance("a", &[("env", "prod")])];
    assert!(
        matching_instances(&LabelSelector::default(), all)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn instance_is_ready_reads_condition() {
    let mut inst = (*instance("a", &[])).clone();
    assert!(!instance_is_ready(&inst));
    inst.status = Some(MicromegasInstanceStatus {
        observed_generation: None,
        conditions: vec![micromegas_operator::conditions::ready(
            true,
            micromegas_operator::conditions::reasons::CONNECTED,
            "",
            None,
        )],
    });
    assert!(instance_is_ready(&inst));
}

// Tests from src/reconcile/mod.rs

#[test]
fn backoff_doubles_and_caps() {
    let b = Backoff::default();
    assert_eq!(b.next("k"), Duration::from_secs(30));
    assert_eq!(b.next("k"), Duration::from_secs(60));
    assert_eq!(b.next("k"), Duration::from_secs(120));
    for _ in 0..10 {
        b.next("k");
    }
    assert_eq!(b.next("k"), Duration::from_secs(600));
    b.reset("k");
    assert_eq!(b.next("k"), Duration::from_secs(30));
}
