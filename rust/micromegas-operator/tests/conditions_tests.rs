//! Tests for micromegas_operator::conditions.

use micromegas_operator::conditions::{is_ready, ready, reasons, upsert};

#[test]
fn upsert_appends_new_type() {
    let mut list = Vec::new();
    upsert(&mut list, ready(true, reasons::SYNCED, "ok", Some(1)));
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].status, "True");
}

#[test]
fn upsert_keeps_transition_time_when_status_unchanged() {
    let mut list = vec![ready(false, reasons::API_ERROR, "boom", Some(1))];
    let first_time = list[0].last_transition_time.clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    upsert(&mut list, ready(false, reasons::CONFLICT, "other", Some(2)));
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, reasons::CONFLICT);
    assert_eq!(list[0].last_transition_time, first_time);
}

#[test]
fn upsert_moves_transition_time_when_status_flips() {
    let mut list = vec![ready(false, reasons::API_ERROR, "boom", Some(1))];
    let first_time = list[0].last_transition_time.clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    upsert(&mut list, ready(true, reasons::SYNCED, "ok", Some(2)));
    assert_ne!(list[0].last_transition_time, first_time);
    assert!(is_ready(&list));
}

#[test]
fn is_ready_false_when_absent() {
    assert!(!is_ready(&[]));
}
