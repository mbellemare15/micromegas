pub mod instance;

use crate::auth::TokenCache;
use crate::crds::MicromegasInstance;
use kube::api::{Patch, PatchParams};
use kube::runtime::events::Recorder;
use kube::runtime::reflector::Store;
use kube::{Api, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FIELD_MANAGER: &str = "micromegas-operator";

pub struct Context {
    pub client: kube::Client,
    pub http: reqwest::Client,
    pub cluster_name: String,
    pub instances: Store<MicromegasInstance>,
    /// instance uid -> (credentials fingerprint, cache). A changed fingerprint means the Secret rotated.
    pub tokens: Mutex<HashMap<String, (String, Arc<TokenCache>)>>,
    pub recorder: Recorder,
    pub backoff: Backoff,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kubernetes API: {0}")]
    Kube(#[from] kube::Error),
    #[error("finalizer: {0}")]
    Finalizer(String),
    #[error("transient: {0}")]
    Transient(String),
}

/// Per-object exponential requeue: 30s, 60s, ... capped at 10m. kube-runtime
/// has no built-in backoff for error_policy.
#[derive(Default)]
pub struct Backoff {
    attempts: Mutex<HashMap<String, u32>>,
}

impl Backoff {
    const BASE: Duration = Duration::from_secs(30);
    const CAP: Duration = Duration::from_secs(600);

    pub fn next(&self, key: &str) -> Duration {
        let mut attempts = self.attempts.lock().expect("backoff mutex");
        let n = attempts.entry(key.to_string()).or_insert(0);
        let delay = Self::BASE.saturating_mul(1u32 << (*n).min(5));
        *n += 1;
        delay.min(Self::CAP)
    }

    pub fn reset(&self, key: &str) {
        self.attempts.lock().expect("backoff mutex").remove(key);
    }
}

pub async fn patch_status<K>(
    api: &Api<K>,
    name: &str,
    status: &impl Serialize,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + Debug,
{
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::Backoff;
    use std::time::Duration;

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
}
