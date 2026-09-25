use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, LabelSelector};
// The KubeSchema derive parses `Rule::new(...)` inside `#[x_kube(validation = ...)]`
// as a DSL at macro-expansion time; it never emits code referencing this type.
#[allow(unused_imports)]
use kube::core::cel::Rule;
use kube::{CustomResource, KubeSchema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_SCREEN_TYPE: &str = "notebook";

#[derive(CustomResource, KubeSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "micromegas.info",
    version = "v1alpha1",
    kind = "Screen",
    plural = "screens",
    namespaced,
    status = "ScreenStatus",
    printcolumn = r#"{"name":"Folder","type":"string","jsonPath":".spec.folderPath"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].status"}"#,
    printcolumn = r#"{"name":"Reason","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].reason"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
#[x_kube(validation = Rule::new("has(self.config) != has(self.configFrom)")
    .message("exactly one of config or configFrom must be set"))]
pub struct ScreenSpec {
    /// Selects the MicromegasInstance objects this screen is written to.
    pub instance_selector: LabelSelector,
    /// Server-side screen name. Defaults to metadata.name. Must satisfy the server's slug rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[x_kube(validation = Rule::new("self == oldSelf").message("name is immutable"))]
    pub name: Option<String>,
    #[serde(default = "default_screen_type")]
    #[x_kube(validation = Rule::new("self == oldSelf").message("screenType is immutable"))]
    pub screen_type: String,
    /// Folder path such as "team/prod". Empty means root. The folder is created implicitly.
    #[serde(default)]
    pub folder_path: String,
    /// Notebook config, same shape as the REST API's config field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "preserve_unknown_fields")]
    pub config: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_from: Option<ConfigFrom>,
}

fn default_screen_type() -> String {
    DEFAULT_SCREEN_TYPE.to_string()
}

// schemars renders serde_json::Value as an unconstrained schema, which the
// apiserver rejects as non-structural; the CRD needs the explicit marker.
fn preserve_unknown_fields(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "object",
        "x-kubernetes-preserve-unknown-fields": true
    })
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFrom {
    pub config_map_key_ref: ConfigMapKeyRef,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigMapKeyRef {
    /// ConfigMap in the Screen's namespace.
    pub name: String,
    pub key: String,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub instances: Vec<ScreenInstanceStatus>,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInstanceStatus {
    pub name: String,
    pub namespace: String,
    pub screen_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
