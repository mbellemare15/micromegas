use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::{CustomResource, KubeSchema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RESYNC_INTERVAL: &str = "10m";

#[derive(CustomResource, KubeSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "micromegas.info",
    version = "v1alpha1",
    kind = "MicromegasInstance",
    plural = "micromegasinstances",
    shortname = "mmi",
    namespaced,
    status = "MicromegasInstanceStatus",
    printcolumn = r#"{"name":"URL","type":"string","jsonPath":".spec.url"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct MicromegasInstanceSpec {
    /// Base URL of the analytics web app, including MICROMEGAS_BASE_PATH if any.
    pub url: String,
    /// Omitted means no Authorization header; only valid against a server run with --disable-auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthSpec>,
    /// How often managed screens are re-compared with the server, e.g. "10m". Minimum "1m".
    #[serde(default = "default_resync_interval")]
    pub resync_interval: String,
}

fn default_resync_interval() -> String {
    DEFAULT_RESYNC_INTERVAL.to_string()
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthSpec {
    pub oidc_client_credentials: OidcClientCredentialsSpec,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OidcClientCredentialsSpec {
    /// OIDC issuer URL; its discovery document supplies the token endpoint.
    pub issuer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
    pub secret_ref: SecretKeyRef,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    /// Secret in the instance's namespace.
    pub name: String,
    #[serde(default = "default_client_id_key")]
    pub client_id_key: String,
    #[serde(default = "default_client_secret_key")]
    pub client_secret_key: String,
}

fn default_client_id_key() -> String {
    "client_id".to_string()
}

fn default_client_secret_key() -> String {
    "client_secret".to_string()
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MicromegasInstanceStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
