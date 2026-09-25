use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};
use k8s_openapi::jiff::Timestamp;

pub const READY: &str = "Ready";

pub mod reasons {
    pub const SYNCED: &str = "Synced";
    pub const CONFLICT: &str = "Conflict";
    pub const NO_MATCHING_INSTANCE: &str = "NoMatchingInstance";
    pub const INSTANCE_NOT_READY: &str = "InstanceNotReady";
    pub const INVALID_NAME: &str = "InvalidName";
    pub const INVALID_CONFIG: &str = "InvalidConfig";
    pub const CONFIG_MAP_NOT_FOUND: &str = "ConfigMapNotFound";
    pub const API_ERROR: &str = "ApiError";
    pub const CONNECTED: &str = "Connected";
    pub const SECRET_NOT_FOUND: &str = "SecretNotFound";
    pub const TOKEN_ERROR: &str = "TokenError";
    pub const UNREACHABLE: &str = "Unreachable";
    pub const UNAUTHORIZED: &str = "Unauthorized";
    pub const INVALID_SPEC: &str = "InvalidSpec";
}

pub fn ready(ok: bool, reason: &str, message: &str, observed_generation: Option<i64>) -> Condition {
    Condition {
        type_: READY.to_string(),
        status: if ok { "True" } else { "False" }.to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation,
        last_transition_time: Time(Timestamp::now()),
    }
}

/// Kubernetes convention: lastTransitionTime moves only when `status` changes,
/// so consumers can tell "still failing since X" from "failed again".
pub fn upsert(conditions: &mut Vec<Condition>, mut new: Condition) {
    match conditions.iter_mut().find(|c| c.type_ == new.type_) {
        Some(existing) => {
            if existing.status == new.status {
                new.last_transition_time = existing.last_transition_time.clone();
            }
            *existing = new;
        }
        None => conditions.push(new),
    }
}

pub fn is_ready(conditions: &[Condition]) -> bool {
    conditions
        .iter()
        .any(|c| c.type_ == READY && c.status == "True")
}
