# Kubernetes Operator for Screens — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `micromegas-operator` binary, Helm chart, and container image that reconcile `Screen` and `MicromegasInstance` custom resources into notebook screens on an analytics-web-srv, with the shared wire types extracted into a crate both sides use.

**Architecture:** Two kube-rs controllers in one process. The instance controller probes each `MicromegasInstance` (Secret → OIDC client-credentials token → `GET /api/screen-types`) and writes a `Ready` condition. The screen controller resolves the desired screen from the CR (inline config or ConfigMap key), matches instances by label selector, and runs a pure `plan()` against `GET /api/screens/{name}` that yields create / update / no-op / conflict. Ownership is the server's existing `managed_by` column stamped `k8s://<cluster>/<namespace>/<name>`; a finalizer deletes on CR removal.

**Tech Stack:** Rust 2024 edition, kube 4.2 (`runtime`, `derive`), k8s-openapi 0.28 (`v1_32`, `schemars`), schemars 1, reqwest 0.12 (rustls), wiremock 0.6, serde_norway for CRD YAML, humantime, axum for probes, `micromegas_main` for telemetry. Helm 3 chart. Python (poetry venv in `python/micromegas`) for the end-to-end script.

**Spec:** `tasks/backlog/k8s_operator_plan.md`

## Global Constraints

- Unix line endings in every file. Comments explain *why*, never *what*. No issue numbers or plan-section citations in code comments or `mkdocs/` docs (CHANGELOG only).
- Never push, never commit to `main`. Work on branch `k8s-operator` (create from `k8s-operator-design`). Local commits after every task.
- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo machete`, `cargo deny check licenses bans sources` and `cargo test` must pass from `rust/` after every task (this is what `build/rust_ci.py native` runs). If `cargo deny` reports a new duplicate crate version pulled in by kube, add it to `bans.skip` in `rust/deny.toml` with a one-line reason, as the existing entries do.
- No live-cluster or live-DB tests are checked in. Anything needing kind or a running server is the manual script in Task 12.
- API group `micromegas.info`, version `v1alpha1`. Kinds `MicromegasInstance` (plural `micromegasinstances`, shortname `mmi`) and `Screen` (plural `screens`). Finalizer `micromegas.info/screen`. `managed_by` format `k8s://<cluster-name>/<namespace>/<cr-name>`.
- Reason strings used in `Ready` conditions, verbatim: `Synced`, `Conflict`, `NoMatchingInstance`, `InstanceNotReady`, `InvalidName`, `InvalidConfig`, `ConfigMapNotFound`, `ApiError`, `Connected`, `SecretNotFound`, `TokenError`, `Unreachable`, `Unauthorized`, `InvalidSpec`.
- Default resync interval `10m`, minimum `1m`. Token refresh margin 60 s.
- Deviation from spec, deliberate: v1 watches either all namespaces or exactly one (`--watch-namespace`), not a list. The Helm value is `watchNamespace`. A list requires one controller per namespace and is a follow-up.
- Rust API breaks are fine and are recorded in `CHANGELOG.md` under **Minor breaking change**.

## Review Focus

Inputs the spec implies but a reader would expect to work; each has a pinned test in the owning task.

1. A `Screen` CR whose `metadata.name` is not a valid server slug (`my_screen`, `ab`, `new`) with `spec.name` omitted → `Ready=False / InvalidName`, no HTTP call. Pinned in Task 8 (`desired_identity_rejects_invalid_default_name`).
2. `MicromegasInstance.spec.url` with a trailing slash or a base path (`https://h/telemetry/`) → requests go to `https://h/telemetry/api/screens`, never `//api`. Pinned in Task 6 (`base_url_with_trailing_slash_and_base_path`).
3. A ConfigMap key containing invalid JSON, or valid JSON that is not an object → `InvalidConfig` with the parse message, not a panic. Pinned in Task 8 (`configmap_value_must_be_json_object`).
4. Two `Screen` CRs in different namespaces resolving to the same server screen name → the second sees `Conflict` naming the first's `managed_by`. Pinned in Task 4 (`conflict_when_owned_by_other_cr_same_cluster`).
5. `resyncInterval: 30s` (below the minimum) or `resyncInterval: soon` → instance `Ready=False / InvalidSpec`, never a panic or a hot loop. Pinned in Task 7 (`resync_interval_enforces_minimum`, `resync_interval_rejects_garbage`).

---

## File Structure

```
rust/Cargo.toml                                   # + workspace deps: analytics-web-api, humantime, k8s-openapi, kube, schemars, serde_norway
rust/analytics-web-api/                           # NEW lib crate (Task 1)
  Cargo.toml
  src/lib.rs                                      # re-exports
  src/screen_type.rs                              # moved from analytics-web-srv/src/screen_types.rs
  src/screens.rs                                  # Screen, CreateScreenRequest, UpdateScreenRequest, ErrorResponse
  src/validation.rs                               # ValidationError, normalize_name, validate_name, validate_folder_path
  tests/validation_tests.rs                       # moved from analytics-web-srv/tests/models_tests.rs
  tests/screen_type_tests.rs                      # moved from analytics-web-srv/tests/screen_types_tests.rs
rust/analytics-web-srv/                           # imports the shared crate (Task 1)
  Cargo.toml, src/app_db/models.rs, src/screen_types.rs, src/screens.rs
rust/micromegas-operator/                         # NEW (Tasks 2–9)
  Cargo.toml
  src/lib.rs                                      # pub mod crds, conditions, plan, auth, client, reconcile, health
  src/main.rs                                     # CLI, controllers wiring
  src/bin/crdgen.rs                               # writes/checks CRD YAML
  src/crds/mod.rs, instance.rs, screen.rs
  src/conditions.rs
  src/plan.rs
  src/auth.rs
  src/client.rs
  src/reconcile/mod.rs                            # Context, Error, Backoff, patch_status
  src/reconcile/instance.rs
  src/reconcile/screen.rs
  src/health.rs
  tests/crd_schema.rs
charts/micromegas-operator/                       # NEW (Tasks 2, 10)
  Chart.yaml, values.yaml, README.md
  crds/micromegasinstances.micromegas.info.yaml   # generated
  crds/screens.micromegas.info.yaml               # generated
  templates/_helpers.tpl, deployment.yaml, serviceaccount.yaml, rbac.yaml
  examples/instance.yaml, screen-inline.yaml, screen-configmap.yaml
docker/operator.Dockerfile                        # Task 11
build/build_docker_images.py, docker/README.md    # Task 11
build/rust_ci.py, .github/workflows/rust.yml      # Task 2 (CRD freshness step, charts/** path)
local_test_env/ai_scripts/operator_e2e.py         # Task 12
mkdocs/docs/admin/kubernetes-operator.md          # Task 13
mkdocs/mkdocs.yml, mkdocs/docs/web-app/notebooks/screens-as-code.md, CHANGELOG.md
```

---

### Task 0: Branch

- [ ] **Step 1: Create the working branch from the design branch**

```bash
cd /Users/mbellemare/Code/public/micromegas
git checkout k8s-operator-design
git checkout -b k8s-operator
```

---

### Task 1: Extract `analytics-web-api` shared crate

**Files:**
- Create: `rust/analytics-web-api/Cargo.toml`, `rust/analytics-web-api/src/lib.rs`, `rust/analytics-web-api/src/screens.rs`, `rust/analytics-web-api/src/validation.rs`
- Move: `rust/analytics-web-srv/src/screen_types.rs` → `rust/analytics-web-api/src/screen_type.rs`
- Move: `rust/analytics-web-srv/tests/models_tests.rs` → `rust/analytics-web-api/tests/validation_tests.rs`
- Move: `rust/analytics-web-srv/tests/screen_types_tests.rs` → `rust/analytics-web-api/tests/screen_type_tests.rs`
- Modify: `rust/Cargo.toml` (workspace deps), `rust/analytics-web-srv/Cargo.toml`, `rust/analytics-web-srv/src/app_db/models.rs`, `rust/analytics-web-srv/src/screen_types.rs` (recreated as re-export), `rust/analytics-web-srv/src/screens.rs:19-33`

**Interfaces:**
- Produces (crate `analytics_web_api`): `Screen { name, screen_type, config: serde_json::Value, created_by: Option<String>, updated_by: Option<String>, created_at: Option<DateTime<Utc>>, updated_at: Option<DateTime<Utc>>, managed_by: Option<String>, folder_path: String }`; `CreateScreenRequest { name, screen_type, config, managed_by: Option<String>, folder_path: String }`; `UpdateScreenRequest { config: Option<Value>, managed_by: Option<String>, folder_path: Option<String> }`; `ErrorResponse { code: String, message: String }` with `ErrorResponse::new(&str, &str)`; `ScreenType` with `FromStr`, `as_str()`, `all()`, `info()`, `default_config()`; `ValidationError { code, message }`; `fn validate_name(&str) -> Result<(), ValidationError>`; `fn validate_folder_path(&str) -> Result<(), ValidationError>`; `fn normalize_name(&str) -> String`. All structs derive `Serialize, Deserialize, Clone, Debug, PartialEq`; `Screen` additionally derives `sqlx::FromRow` behind feature `sqlx`.

- [ ] **Step 1: Add the workspace dependency**

In `rust/Cargo.toml` under `[workspace.dependencies]`, after the `micromegas = { path = "public", ... }` line, add:

```toml
analytics-web-api = { path = "analytics-web-api", version = "0.32.0" }
```

- [ ] **Step 2: Create the crate manifest**

`rust/analytics-web-api/Cargo.toml`:

```toml
[package]
name = "analytics-web-api"
description = "Wire types and validation rules of the analytics-web-srv REST API, shared with its clients"
keywords.workspace = true
categories.workspace = true
version.workspace = true
edition.workspace = true
homepage.workspace = true
repository.workspace = true
license.workspace = true
authors.workspace = true

[features]
sqlx = ["dep:sqlx"]

[dependencies]
chrono.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx = { workspace = true, optional = true }
```

- [ ] **Step 3: Move the screen type module**

```bash
cd /Users/mbellemare/Code/public/micromegas/rust
mkdir -p analytics-web-api/src analytics-web-api/tests
git mv analytics-web-srv/src/screen_types.rs analytics-web-api/src/screen_type.rs
git mv analytics-web-srv/tests/models_tests.rs analytics-web-api/tests/validation_tests.rs
git mv analytics-web-srv/tests/screen_types_tests.rs analytics-web-api/tests/screen_type_tests.rs
```

`screen_type.rs` needs no edits: it only uses `serde`, `serde_json`, `std`.

- [ ] **Step 4: Write `validation.rs`**

Cut these items out of `rust/analytics-web-srv/src/app_db/models.rs` and paste them into `rust/analytics-web-api/src/validation.rs`: `RESERVED_NAMES`, `ValidationError` + its `impl`, `normalize_name`, `validate_name_core`, `validate_name`, `MAX_FOLDER_PATH_LENGTH`, `validate_folder_segment`, `validate_folder_path`. Leave `expand_path_prefixes` in `models.rs` (it exists for advisory locking, a server concern). The file starts with:

```rust
//! Name and folder-path rules enforced by analytics-web-srv. Clients validate
//! with the same code so a rejected request is caught before it is sent.

use serde::{Deserialize, Serialize};

/// Reserved names that cannot be used.
const RESERVED_NAMES: &[&str] = &["new"];

/// Validation error for screen names and folder paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}
```

followed by the moved functions unchanged.

- [ ] **Step 5: Write `screens.rs`**

`rust/analytics-web-api/src/screens.rs`:

```rust
use crate::validation::ValidationError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A screen as returned by `GET /api/screens/{name}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct Screen {
    pub name: String,
    pub screen_type: String,
    pub config: serde_json::Value,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub managed_by: Option<String>,
    pub folder_path: String,
}

/// Body of `POST /api/screens`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateScreenRequest {
    pub name: String,
    pub screen_type: String,
    pub config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,
    #[serde(default)]
    pub folder_path: String,
}

/// Body of `PUT /api/screens/{name}`. Every field is optional; the server keeps
/// the current value for anything omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateScreenRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
}

/// Error body returned by every `/api/screens` route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl From<ValidationError> for ErrorResponse {
    fn from(err: ValidationError) -> Self {
        Self {
            code: err.code,
            message: err.message,
        }
    }
}
```

- [ ] **Step 6: Write `lib.rs`**

```rust
//! Wire types and validation rules of the analytics-web-srv REST API.
//!
//! The server and its Rust clients (the Kubernetes operator) both depend on
//! this crate so a request built here is always one the server accepts.

pub mod screen_type;
pub mod screens;
pub mod validation;

pub use screen_type::{ParseScreenTypeError, ScreenType, ScreenTypeInfo};
pub use screens::{CreateScreenRequest, ErrorResponse, Screen, UpdateScreenRequest};
pub use validation::{ValidationError, normalize_name, validate_folder_path, validate_name};
```

- [ ] **Step 7: Fix the moved tests' imports**

In `rust/analytics-web-api/tests/validation_tests.rs` replace
`use analytics_web_srv::app_db::{normalize_name, validate_folder_path, validate_name};`
with
`use analytics_web_api::{normalize_name, validate_folder_path, validate_name};`.

In `rust/analytics-web-api/tests/screen_type_tests.rs` replace
`use analytics_web_srv::screen_types::ScreenType;`
with
`use analytics_web_api::ScreenType;`.

Update the `//!` doc line at the top of each file to name the new crate.

- [ ] **Step 8: Run the new crate's tests**

Run: `cd rust && cargo test -p analytics-web-api`
Expected: PASS, every moved test green.

- [ ] **Step 9: Point the server at the shared crate**

`rust/analytics-web-srv/Cargo.toml`, under `[dependencies]` right after the `micromegas = ...` line:

```toml
analytics-web-api = { workspace = true, features = ["sqlx"] }
```

`rust/analytics-web-srv/src/app_db/models.rs`: delete the `Screen`, `CreateScreenRequest`, `UpdateScreenRequest`, `ValidationError`, `RESERVED_NAMES`, `normalize_name`, `validate_name_core`, `validate_name`, `MAX_FOLDER_PATH_LENGTH`, `validate_folder_segment`, `validate_folder_path` definitions (already moved). Add at the top, after the existing `use` lines:

```rust
pub use analytics_web_api::{
    CreateScreenRequest, Screen, UpdateScreenRequest, ValidationError, normalize_name,
    validate_folder_path, validate_name,
};
```

`rust/analytics-web-srv/src/app_db/mod.rs` needs no change: its `pub use models::{...}` list resolves through the re-exports.

Recreate `rust/analytics-web-srv/src/screen_types.rs` with only:

```rust
pub use analytics_web_api::screen_type::{ParseScreenTypeError, ScreenType, ScreenTypeInfo};
```

`rust/analytics-web-srv/src/screens.rs`: delete the local `ErrorResponse` struct and its `impl` (lines 19–33) and add `use analytics_web_api::ErrorResponse;` to the imports. Every `ErrorResponse::new(...)` call keeps working. The other `ErrorResponse` copies in `folders.rs`, `groups.rs`, etc. are out of scope; leave them.

- [ ] **Step 10: Build and test the server**

Run: `cd rust && cargo build -p analytics-web-srv && cargo test -p analytics-web-srv`
Expected: builds; non-ignored tests PASS. (`screens_tests` is `#[ignore]`, live-DB.)

- [ ] **Step 11: Workspace-wide checks**

Run: `cd rust && cargo fmt && cargo clippy --workspace -- -D warnings && cargo machete`
Expected: no warnings. If `cargo machete` flags `chrono` in `analytics-web-api`, it is used by `Screen`; if it flags anything in `analytics-web-srv`, remove that dependency line.

- [ ] **Step 12: Commit**

```bash
git add -A rust/analytics-web-api rust/analytics-web-srv rust/Cargo.toml rust/Cargo.lock
git commit -m "Extract analytics-web-api crate with screen wire types and validators"
```

---

### Task 2: Operator crate scaffold, CRD types, crdgen, committed CRDs, CI freshness check

**Files:**
- Create: `rust/micromegas-operator/Cargo.toml`, `src/lib.rs`, `src/main.rs` (placeholder), `src/bin/crdgen.rs`, `src/crds/mod.rs`, `src/crds/instance.rs`, `src/crds/screen.rs`, `tests/crd_schema.rs`
- Create: `charts/micromegas-operator/crds/*.yaml` (generated)
- Modify: `rust/Cargo.toml` (workspace deps), `build/rust_ci.py`, `.github/workflows/rust.yml`

**Interfaces:**
- Produces: `crds::{MicromegasInstance, MicromegasInstanceSpec, MicromegasInstanceStatus, AuthSpec, OidcClientCredentialsSpec, SecretKeyRef}` and `crds::{Screen, ScreenSpec, ScreenStatus, ScreenInstanceStatus, ConfigFrom, ConfigMapKeyRef}` (exact fields below). `MicromegasInstanceSpec::resync_interval: String`. `ScreenSpec::instance_selector: LabelSelector`, `name: Option<String>`, `screen_type: String`, `folder_path: String`, `config: Option<serde_json::Value>`, `config_from: Option<ConfigFrom>`.

- [ ] **Step 1: Add workspace dependencies**

In `rust/Cargo.toml` `[workspace.dependencies]` (alphabetical position):

```toml
humantime = "2.4"
k8s-openapi = { version = "0.28", features = ["v1_32", "schemars"] }
kube = { version = "4.2", features = ["runtime", "derive"] }
schemars = "1"
serde_norway = "0.9"
```

If `kube::core::cel::Rule` (used in Step 5) is not found with these features, add `"cel"` to the kube feature list; it is the feature that gates the CEL module in some kube releases.

- [ ] **Step 2: Crate manifest**

`rust/micromegas-operator/Cargo.toml`:

```toml
[package]
name = "micromegas-operator"
description = "Kubernetes operator that reconciles Screen custom resources into analytics-web-srv screens"
keywords.workspace = true
categories.workspace = true
version.workspace = true
edition.workspace = true
homepage.workspace = true
repository.workspace = true
license.workspace = true
authors.workspace = true
default-run = "micromegas-operator"

[dependencies]
analytics-web-api.workspace = true
micromegas.workspace = true

anyhow.workspace = true
axum.workspace = true
chrono.workspace = true
clap.workspace = true
futures.workspace = true
humantime.workspace = true
k8s-openapi.workspace = true
kube.workspace = true
reqwest.workspace = true
schemars.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_norway.workspace = true
sha2.workspace = true
thiserror.workspace = true
tokio.workspace = true

[dev-dependencies]
wiremock.workspace = true

[lib]
name = "micromegas_operator"
path = "src/lib.rs"

[[bin]]
name = "micromegas-operator"
path = "src/main.rs"

[[bin]]
name = "crdgen"
path = "src/bin/crdgen.rs"
```

- [ ] **Step 3: lib.rs and placeholder main.rs**

`src/lib.rs`:

```rust
//! Kubernetes operator for Micromegas screens.

pub mod crds;
```

(Later tasks append `pub mod conditions;`, `pub mod plan;`, `pub mod auth;`, `pub mod client;`, `pub mod reconcile;`, `pub mod health;`.)

`src/main.rs` placeholder so the crate builds:

```rust
fn main() {
    println!("micromegas-operator: wiring lands in a later task");
}
```

- [ ] **Step 4: CRD types — instance**

`src/crds/mod.rs`:

```rust
mod instance;
mod screen;

pub use instance::{
    AuthSpec, MicromegasInstance, MicromegasInstanceSpec, MicromegasInstanceStatus,
    OidcClientCredentialsSpec, SecretKeyRef,
};
pub use screen::{ConfigFrom, ConfigMapKeyRef, Screen, ScreenInstanceStatus, ScreenSpec, ScreenStatus};

pub const GROUP: &str = "micromegas.info";
```

`src/crds/instance.rs`:

```rust
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
```

If the derive rejects the JSON-string `printcolumn` form, use the structured form kube 4 documents: `printcolumn(name = "URL", type_ = "string", json_path = ".spec.url")`.

- [ ] **Step 5: CRD types — screen**

`src/crds/screen.rs`:

```rust
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, LabelSelector};
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
```

- [ ] **Step 6: Write the failing CRD schema test**

`rust/micromegas-operator/tests/crd_schema.rs`:

```rust
use kube::CustomResourceExt;
use micromegas_operator::crds::{MicromegasInstance, Screen};

fn schema_of<K: CustomResourceExt>() -> serde_json::Value {
    let crd = serde_json::to_value(K::crd()).unwrap();
    crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"].clone()
}

#[test]
fn screen_crd_identity() {
    let crd = Screen::crd();
    assert_eq!(crd.spec.group, "micromegas.info");
    assert_eq!(crd.spec.names.kind, "Screen");
    assert_eq!(crd.spec.names.plural, "screens");
    assert_eq!(crd.spec.scope, "Namespaced");
    assert!(crd.spec.versions[0].subresources.as_ref().unwrap().status.is_some());
}

#[test]
fn screen_config_preserves_unknown_fields() {
    let schema = schema_of::<Screen>();
    let config = &schema["properties"]["spec"]["properties"]["config"];
    assert_eq!(config["x-kubernetes-preserve-unknown-fields"], true);
}

#[test]
fn screen_spec_has_one_of_rule_and_immutability_rules() {
    let schema = schema_of::<Screen>();
    let spec = &schema["properties"]["spec"];
    let rules = spec["x-kubernetes-validations"].as_array().unwrap();
    assert!(rules.iter().any(|r| r["rule"].as_str().unwrap().contains("has(self.config)")));
    let screen_type_rules = spec["properties"]["screenType"]["x-kubernetes-validations"].as_array().unwrap();
    assert_eq!(screen_type_rules[0]["rule"], "self == oldSelf");
}

#[test]
fn screen_defaults_apply_on_deserialize() {
    let yaml = r#"
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata: { name: demo, namespace: default }
spec:
  instanceSelector: { matchLabels: { env: dev } }
  config: { cells: [] }
"#;
    let screen: Screen = serde_norway::from_str(yaml).unwrap();
    assert_eq!(screen.spec.screen_type, "notebook");
    assert_eq!(screen.spec.folder_path, "");
    assert!(screen.spec.name.is_none());
}

#[test]
fn instance_crd_identity_and_defaults() {
    let crd = MicromegasInstance::crd();
    assert_eq!(crd.spec.names.plural, "micromegasinstances");
    assert_eq!(crd.spec.names.short_names, Some(vec!["mmi".to_string()]));
    let yaml = r#"
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata: { name: dev, namespace: default }
spec: { url: "http://127.0.0.1:3000" }
"#;
    let inst: MicromegasInstance = serde_norway::from_str(yaml).unwrap();
    assert_eq!(inst.spec.resync_interval, "10m");
    assert!(inst.spec.auth.is_none());
}
```

- [ ] **Step 7: Run tests to verify they fail, then compile**

Run: `cd rust && cargo test -p micromegas-operator --test crd_schema`
Expected: compile errors until Steps 4–5 compile cleanly; then run again. If `cargo` reports that kube 4.2 requires a different k8s-openapi version, run `cargo tree -p kube -i k8s-openapi -e features` and pin the workspace `k8s-openapi` to that version. Iterate until all five tests PASS.

- [ ] **Step 8: crdgen binary**

`src/bin/crdgen.rs`:

```rust
//! Writes the CRD manifests the Helm chart installs, or verifies they are current.

use clap::Parser;
use kube::CustomResourceExt;
use micromegas_operator::crds::{MicromegasInstance, Screen};
use std::path::PathBuf;

#[derive(Parser)]
#[clap(about = "Generate CRD YAML for charts/micromegas-operator/crds")]
struct Cli {
    /// Directory receiving one file per CRD.
    out_dir: PathBuf,
    /// Exit non-zero if the files on disk differ from the generated output.
    #[clap(long)]
    check: bool,
}

fn render() -> Vec<(&'static str, String)> {
    vec![
        (
            "micromegasinstances.micromegas.info.yaml",
            serde_norway::to_string(&MicromegasInstance::crd()).expect("serialize CRD"),
        ),
        (
            "screens.micromegas.info.yaml",
            serde_norway::to_string(&Screen::crd()).expect("serialize CRD"),
        ),
    ]
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut stale = Vec::new();
    for (file, content) in render() {
        let path = cli.out_dir.join(file);
        if cli.check {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            if on_disk != content {
                stale.push(path.display().to_string());
            }
        } else {
            std::fs::create_dir_all(&cli.out_dir)?;
            std::fs::write(&path, content)?;
            println!("wrote {}", path.display());
        }
    }
    if !stale.is_empty() {
        anyhow::bail!(
            "CRD files are stale: {}. Run: cargo run -p micromegas-operator --bin crdgen -- ../charts/micromegas-operator/crds",
            stale.join(", ")
        );
    }
    Ok(())
}
```

- [ ] **Step 9: Generate and check**

Run:
```bash
cd rust && cargo run -p micromegas-operator --bin crdgen -- ../charts/micromegas-operator/crds
cargo run -p micromegas-operator --bin crdgen -- --check ../charts/micromegas-operator/crds
```
Expected: two files written; second command exits 0. Inspect `screens.micromegas.info.yaml` and confirm `config` shows `x-kubernetes-preserve-unknown-fields: true` and `screenType` shows `x-kubernetes-validations`.

- [ ] **Step 10: CI freshness step and workflow paths**

In `build/rust_ci.py` `run_native()` steps list, after `("Running Tests", "cargo test", None)`:

```python
        (
            "CRD Freshness Check",
            "cargo run -p micromegas-operator --bin crdgen -- --check ../charts/micromegas-operator/crds",
            None,
        ),
```

In `.github/workflows/rust.yml`, add `- 'charts/micromegas-operator/crds/**'` to both `paths:` lists (push and pull_request), after `'rust/**'`.

- [ ] **Step 11: Full checks and commit**

Run: `cd rust && cargo fmt && cargo clippy --workspace -- -D warnings && cargo machete && cargo deny check licenses bans sources && cargo test -p micromegas-operator`
Expected: clean. Fix any `bans.skip` additions in `rust/deny.toml` with a one-line comment naming the crate that pulls the duplicate.

```bash
git add rust/micromegas-operator rust/Cargo.toml rust/Cargo.lock rust/deny.toml charts build/rust_ci.py .github/workflows/rust.yml
git commit -m "Add micromegas-operator crate with CRD types and generated manifests"
```

---

### Task 3: Condition helpers

**Files:**
- Create: `rust/micromegas-operator/src/conditions.rs`
- Modify: `src/lib.rs` (add `pub mod conditions;`)

**Interfaces:**
- Produces: `conditions::READY: &str`; `conditions::reasons::{SYNCED, CONFLICT, NO_MATCHING_INSTANCE, INSTANCE_NOT_READY, INVALID_NAME, INVALID_CONFIG, CONFIG_MAP_NOT_FOUND, API_ERROR, CONNECTED, SECRET_NOT_FOUND, TOKEN_ERROR, UNREACHABLE, UNAUTHORIZED, INVALID_SPEC}`; `fn ready(ok: bool, reason: &str, message: &str, observed_generation: Option<i64>) -> Condition`; `fn upsert(conditions: &mut Vec<Condition>, new: Condition)`; `fn is_ready(conditions: &[Condition]) -> bool`.

- [ ] **Step 1: Write the failing tests** (module tests at the bottom of `conditions.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator conditions`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement**

`src/conditions.rs`:

```rust
use chrono::Utc;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};

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
        last_transition_time: Time(Utc::now()),
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
```

Add `pub mod conditions;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cd rust && cargo test -p micromegas-operator conditions`
Expected: 4 PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/micromegas-operator
git commit -m "Add Ready condition helpers to micromegas-operator"
```

---

### Task 4: Pure planning: desired vs current

**Files:**
- Create: `rust/micromegas-operator/src/plan.rs`
- Modify: `src/lib.rs` (add `pub mod plan;`)

**Interfaces:**
- Consumes: `analytics_web_api::{Screen, CreateScreenRequest, UpdateScreenRequest}`.
- Produces: `plan::Desired { name: String, screen_type: String, folder_path: String, config: Value, managed_by: String }`; `plan::Action { Create(CreateScreenRequest), Update(UpdateScreenRequest), NoOp, Conflict(String) }` (derives `Debug, PartialEq`); `fn plan(desired: &Desired, current: Option<&Screen>) -> Action`; `fn managed_by(cluster: &str, namespace: &str, name: &str) -> String`; `fn canonical(value: &Value) -> String`; `fn config_hash(value: &Value) -> String` (returns `sha256:<hex>`).

- [ ] **Step 1: Write the failing tests** (bottom of `plan.rs`)

```rust
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
                assert_eq!(req.managed_by.as_deref(), Some("k8s://prod-eu/game-system/overview"));
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
        let c = current(Some("https://github.com/org/dashboards.git"), d.config.clone(), "team/prod");
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator plan`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/plan.rs`:

```rust
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
            return Action::Conflict(format!(
                "screen '{}' is managed by '{owner}'",
                current.name
            ));
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
```

Note: `Value::Object(...)` from an iterator of `(String, Value)` requires `serde_json::Map: FromIterator`, which it implements. If `preserve_order` is enabled somewhere, the collected map keeps the BTreeMap's sorted order, so the output is still canonical.

Add `pub mod plan;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cd rust && cargo test -p micromegas-operator plan`
Expected: 10 PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/micromegas-operator
git commit -m "Add pure screen planning with managed_by ownership rules"
```

---

### Task 5: OIDC client-credentials token cache

**Files:**
- Create: `rust/micromegas-operator/src/auth.rs`
- Modify: `src/lib.rs` (add `pub mod auth;`)

**Interfaces:**
- Produces: `auth::OidcClientCredentials { issuer: String, client_id: String, client_secret: String, audience: Option<String> }`; `auth::TokenCache::new(http: reqwest::Client, creds: OidcClientCredentials) -> TokenCache`; `async fn TokenCache::bearer(&self) -> Result<String, TokenError>`; `auth::TokenError { Discovery(String), Request(String), MalformedResponse }` (thiserror); `fn OidcClientCredentials::fingerprint(&self) -> String`.

- [ ] **Step 1: Write the failing tests** (bottom of `auth.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn issuer(server: &MockServer) -> String {
        Mock::given(method("GET"))
            .and(path("/realm/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_endpoint": format!("{}/realm/token", server.uri())
            })))
            .mount(server)
            .await;
        format!("{}/realm/", server.uri())
    }

    fn creds(issuer: String, audience: Option<&str>) -> OidcClientCredentials {
        OidcClientCredentials {
            issuer,
            client_id: "op".into(),
            client_secret: "s3cret".into(),
            audience: audience.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn fetches_token_with_client_credentials_form() {
        let server = MockServer::start().await;
        let iss = issuer(&server).await;
        Mock::given(method("POST"))
            .and(path("/realm/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("client_id=op"))
            .and(body_string_contains("client_secret=s3cret"))
            .and(body_string_contains("audience=micromegas"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "tok-1", "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        let cache = TokenCache::new(reqwest::Client::new(), creds(iss, Some("micromegas")));
        assert_eq!(cache.bearer().await.unwrap(), "tok-1");
        assert_eq!(cache.bearer().await.unwrap(), "tok-1");
    }

    #[tokio::test]
    async fn refetches_when_inside_refresh_margin() {
        let server = MockServer::start().await;
        let iss = issuer(&server).await;
        Mock::given(method("POST"))
            .and(path("/realm/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "short", "expires_in": 30
            })))
            .expect(2)
            .mount(&server)
            .await;
        let cache = TokenCache::new(reqwest::Client::new(), creds(iss, None));
        cache.bearer().await.unwrap();
        cache.bearer().await.unwrap();
    }

    #[tokio::test]
    async fn token_endpoint_rejection_is_request_error() {
        let server = MockServer::start().await;
        let iss = issuer(&server).await;
        Mock::given(method("POST"))
            .and(path("/realm/token"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid_client"))
            .mount(&server)
            .await;
        let cache = TokenCache::new(reqwest::Client::new(), creds(iss, None));
        assert!(matches!(cache.bearer().await, Err(TokenError::Request(msg)) if msg.contains("401")));
    }

    #[tokio::test]
    async fn missing_discovery_is_discovery_error() {
        let server = MockServer::start().await;
        let cache = TokenCache::new(
            reqwest::Client::new(),
            creds(format!("{}/nowhere", server.uri()), None),
        );
        assert!(matches!(cache.bearer().await, Err(TokenError::Discovery(_))));
    }

    #[test]
    fn fingerprint_changes_with_any_field() {
        let a = creds("https://i".into(), None);
        let mut b = creds("https://i".into(), None);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.client_secret = "other".into();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator auth`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/auth.rs`:

```rust
//! OIDC client-credentials grant, cached per instance. The access token is sent
//! as the bearer, which is what analytics-web-srv validates (same as the Python
//! machine client).

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const DEFAULT_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcClientCredentials {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub audience: Option<String>,
}

impl OidcClientCredentials {
    /// Lets the reconciler notice a rotated Secret without keeping the secret itself around.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        for part in [
            self.issuer.as_str(),
            self.client_id.as_str(),
            self.client_secret.as_str(),
            self.audience.as_deref().unwrap_or(""),
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0u8]);
        }
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("OIDC discovery failed: {0}")]
    Discovery(String),
    #[error("token request failed: {0}")]
    Request(String),
    #[error("token response has no access_token")]
    MalformedResponse,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

pub struct TokenCache {
    http: reqwest::Client,
    creds: OidcClientCredentials,
    // tokio Mutex: held across the await of a refresh so concurrent reconciles
    // share one token request instead of each hitting the IdP.
    state: Mutex<Option<CachedToken>>,
}

#[derive(Deserialize)]
struct Discovery {
    token_endpoint: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
}

impl TokenCache {
    pub fn new(http: reqwest::Client, creds: OidcClientCredentials) -> Self {
        Self {
            http,
            creds,
            state: Mutex::new(None),
        }
    }

    pub async fn bearer(&self) -> Result<String, TokenError> {
        let mut guard = self.state.lock().await;
        if let Some(token) = guard.as_ref()
            && token.expires_at > Instant::now() + REFRESH_MARGIN
        {
            return Ok(token.access_token.clone());
        }
        let fresh = self.fetch().await?;
        let access_token = fresh.access_token.clone();
        *guard = Some(fresh);
        Ok(access_token)
    }

    async fn fetch(&self) -> Result<CachedToken, TokenError> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.creds.issuer.trim_end_matches('/')
        );
        let discovery: Discovery = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| TokenError::Discovery(e.to_string()))?
            .json()
            .await
            .map_err(|e| TokenError::Discovery(e.to_string()))?;

        let mut form = vec![
            ("grant_type", "client_credentials"),
            ("client_id", self.creds.client_id.as_str()),
            ("client_secret", self.creds.client_secret.as_str()),
        ];
        if let Some(audience) = &self.creds.audience {
            form.push(("audience", audience.as_str()));
        }
        let response = self
            .http
            .post(&discovery.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| TokenError::Request(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(TokenError::Request(format!("HTTP {status}: {body}")));
        }
        let body: TokenResponse = response
            .json()
            .await
            .map_err(|e| TokenError::Request(e.to_string()))?;
        let access_token = body.access_token.ok_or(TokenError::MalformedResponse)?;
        let lifetime = body.expires_in.map(Duration::from_secs).unwrap_or(DEFAULT_LIFETIME);
        Ok(CachedToken {
            access_token,
            expires_at: Instant::now() + lifetime,
        })
    }
}
```

The `if let ... && ...` chain is edition-2024 syntax and compiles on the workspace toolchain; if the toolchain rejects it, nest the two conditions.

Add `pub mod auth;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cd rust && cargo test -p micromegas-operator auth`
Expected: 5 PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/micromegas-operator
git commit -m "Add OIDC client-credentials token cache to micromegas-operator"
```

---

### Task 6: Web API client

**Files:**
- Create: `rust/micromegas-operator/src/client.rs`
- Modify: `src/lib.rs` (add `pub mod client;`)

**Interfaces:**
- Consumes: `auth::{TokenCache, TokenError}`, `analytics_web_api::{Screen, CreateScreenRequest, UpdateScreenRequest, ErrorResponse}`.
- Produces: `client::Credentials { None, Oidc(Arc<TokenCache>) }`; `client::WebApiClient::new(http: reqwest::Client, base_url: &str, credentials: Credentials) -> WebApiClient`; `async fn probe(&self) -> Result<(), ApiError>`; `async fn get_screen(&self, name: &str) -> Result<Option<Screen>, ApiError>`; `async fn create_screen(&self, req: &CreateScreenRequest) -> Result<Screen, ApiError>`; `async fn update_screen(&self, name: &str, req: &UpdateScreenRequest) -> Result<Screen, ApiError>`; `async fn delete_screen(&self, name: &str) -> Result<(), ApiError>` (404 is Ok); `fn api_url(&self, path: &str) -> String`; `client::ApiError { Unauthorized, BadRequest(ErrorResponse), NotFound, Token(TokenError), Transient(String) }` with `fn is_transient(&self) -> bool`.

- [ ] **Step 1: Write the failing tests** (bottom of `client.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::OidcClientCredentials;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn screen_json(managed_by: Option<&str>) -> serde_json::Value {
        json!({
            "name": "overview", "screen_type": "notebook", "config": {"cells": []},
            "created_by": "x", "updated_by": "x", "created_at": null, "updated_at": null,
            "managed_by": managed_by, "folder_path": "team"
        })
    }

    fn client(server: &MockServer) -> WebApiClient {
        WebApiClient::new(reqwest::Client::new(), &server.uri(), Credentials::None)
    }

    #[test]
    fn base_url_with_trailing_slash_and_base_path() {
        let c = WebApiClient::new(reqwest::Client::new(), "https://h/telemetry/", Credentials::None);
        assert_eq!(c.api_url("screens"), "https://h/telemetry/api/screens");
        let root = WebApiClient::new(reqwest::Client::new(), "https://h", Credentials::None);
        assert_eq!(root.api_url("screens/x"), "https://h/api/screens/x");
    }

    #[tokio::test]
    async fn get_screen_found_and_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/screens/overview"))
            .respond_with(ResponseTemplate::new(200).set_body_json(screen_json(Some("k8s://c/ns/n"))))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/api/screens/nope"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"code": "NOT_FOUND", "message": "x"})))
            .mount(&server).await;
        let c = client(&server);
        let found = c.get_screen("overview").await.unwrap().unwrap();
        assert_eq!(found.managed_by.as_deref(), Some("k8s://c/ns/n"));
        assert!(c.get_screen("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_posts_body_and_maps_duplicate_to_bad_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/screens"))
            .and(body_partial_json(json!({"name": "overview", "managed_by": "k8s://c/ns/n", "folder_path": "team"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(screen_json(Some("k8s://c/ns/n"))))
            .mount(&server).await;
        let c = client(&server);
        let req = CreateScreenRequest {
            name: "overview".into(), screen_type: "notebook".into(), config: json!({"cells": []}),
            managed_by: Some("k8s://c/ns/n".into()), folder_path: "team".into(),
        };
        assert_eq!(c.create_screen(&req).await.unwrap().name, "overview");

        let server2 = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/screens"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"code": "DUPLICATE_NAME", "message": "exists"})))
            .mount(&server2).await;
        match client(&server2).create_screen(&req).await {
            Err(ApiError::BadRequest(e)) => assert_eq!(e.code, "DUPLICATE_NAME"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_puts_partial_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT")).and(path("/api/screens/overview"))
            .and(body_partial_json(json!({"folder_path": "new"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(screen_json(Some("k8s://c/ns/n"))))
            .mount(&server).await;
        let req = UpdateScreenRequest { config: None, managed_by: None, folder_path: Some("new".into()) };
        client(&server).update_screen("overview", &req).await.unwrap();
    }

    #[tokio::test]
    async fn delete_treats_404_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE")).and(path("/api/screens/overview"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server).await;
        client(&server).delete_screen("overview").await.unwrap();
    }

    #[tokio::test]
    async fn status_mapping() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/screen-types"))
            .respond_with(ResponseTemplate::new(401)).mount(&server).await;
        assert!(matches!(client(&server).probe().await, Err(ApiError::Unauthorized)));

        let server2 = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/screen-types"))
            .respond_with(ResponseTemplate::new(503).set_body_string("down")).mount(&server2).await;
        let err = client(&server2).probe().await.unwrap_err();
        assert!(err.is_transient(), "{err:?}");

        let unreachable = WebApiClient::new(reqwest::Client::new(), "http://127.0.0.1:9", Credentials::None);
        assert!(unreachable.probe().await.unwrap_err().is_transient());
    }

    #[tokio::test]
    async fn sends_bearer_from_token_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/realm/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token_endpoint": format!("{}/realm/token", server.uri())})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/realm/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "tok", "expires_in": 600})))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/api/screen-types")).and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server).await;
        let cache = Arc::new(TokenCache::new(reqwest::Client::new(), OidcClientCredentials {
            issuer: format!("{}/realm", server.uri()), client_id: "a".into(), client_secret: "b".into(), audience: None,
        }));
        let c = WebApiClient::new(reqwest::Client::new(), &server.uri(), Credentials::Oidc(cache));
        c.probe().await.unwrap();
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator client`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/client.rs`:

```rust
//! Thin typed client for the analytics-web-srv screens routes.

use crate::auth::{TokenCache, TokenError};
use analytics_web_api::{CreateScreenRequest, ErrorResponse, Screen, UpdateScreenRequest};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub enum Credentials {
    None,
    Oidc(Arc<TokenCache>),
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{}: {}", .0.code, .0.message)]
    BadRequest(ErrorResponse),
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error("{0}")]
    Transient(String),
}

impl ApiError {
    pub fn is_transient(&self) -> bool {
        matches!(self, ApiError::Transient(_) | ApiError::Token(_))
    }
}

pub struct WebApiClient {
    http: reqwest::Client,
    base_url: String,
    credentials: Credentials,
}

impl WebApiClient {
    pub fn new(http: reqwest::Client, base_url: &str, credentials: Credentials) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            credentials,
        }
    }

    pub fn api_url(&self, path: &str) -> String {
        format!("{}/api/{path}", self.base_url)
    }

    pub async fn probe(&self) -> Result<(), ApiError> {
        self.send::<serde_json::Value>(Method::GET, "screen-types", None::<&()>)
            .await
            .map(|_| ())
    }

    pub async fn get_screen(&self, name: &str) -> Result<Option<Screen>, ApiError> {
        match self.send(Method::GET, &format!("screens/{name}"), None::<&()>).await {
            Ok(screen) => Ok(Some(screen)),
            Err(ApiError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn create_screen(&self, req: &CreateScreenRequest) -> Result<Screen, ApiError> {
        self.send(Method::POST, "screens", Some(req)).await
    }

    pub async fn update_screen(&self, name: &str, req: &UpdateScreenRequest) -> Result<Screen, ApiError> {
        self.send(Method::PUT, &format!("screens/{name}"), Some(req)).await
    }

    pub async fn delete_screen(&self, name: &str) -> Result<(), ApiError> {
        match self
            .send::<serde_json::Value>(Method::DELETE, &format!("screens/{name}"), None::<&()>)
            .await
        {
            Ok(_) | Err(ApiError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<T, ApiError> {
        let mut request = self.http.request(method, self.api_url(path));
        if let Credentials::Oidc(cache) = &self.credentials {
            request = request.bearer_auth(cache.bearer().await?);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Transient(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            if status == StatusCode::NO_CONTENT {
                return serde_json::from_value(serde_json::Value::Null)
                    .map_err(|e| ApiError::Transient(e.to_string()));
            }
            return response
                .json()
                .await
                .map_err(|e| ApiError::Transient(format!("decoding response: {e}")));
        }
        let text = response.text().await.unwrap_or_default();
        Err(classify(status, &text))
    }
}

fn classify(status: StatusCode, body: &str) -> ApiError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ApiError::Unauthorized,
        StatusCode::NOT_FOUND => ApiError::NotFound,
        StatusCode::BAD_REQUEST => ApiError::BadRequest(
            serde_json::from_str(body)
                .unwrap_or_else(|_| ErrorResponse::new("HTTP_400", body)),
        ),
        _ => ApiError::Transient(format!("HTTP {status}: {body}")),
    }
}
```

Add `pub mod client;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cd rust && cargo test -p micromegas-operator client`
Expected: 7 PASS. The `DELETE` 204 path: `serde_json::from_value::<Value>(Null)` yields `Value::Null`, fine.

- [ ] **Step 5: Commit**

```bash
git add rust/micromegas-operator
git commit -m "Add typed analytics-web-srv client to micromegas-operator"
```

---

### Task 7: Reconcile context and the instance controller

**Files:**
- Create: `rust/micromegas-operator/src/reconcile/mod.rs`, `src/reconcile/instance.rs`
- Modify: `src/lib.rs` (add `pub mod reconcile;`)

**Interfaces:**
- Consumes: `crds::*`, `conditions::*`, `auth::*`, `client::*`.
- Produces (`reconcile`): `Context { client: kube::Client, http: reqwest::Client, cluster_name: String, instances: Store<MicromegasInstance>, tokens: std::sync::Mutex<HashMap<String, (String, Arc<TokenCache>)>>, recorder: Recorder, backoff: Backoff }`; `Error { Kube(kube::Error), Finalizer(String), Transient(String) }`; `Backoff::next(&self, key: &str) -> Duration` and `Backoff::reset(&self, key: &str)`; `async fn patch_status<K>(api: &Api<K>, name: &str, status: &impl Serialize) -> Result<(), kube::Error>`.
- Produces (`reconcile::instance`): `async fn reconcile(Arc<MicromegasInstance>, Arc<Context>) -> Result<Action, Error>`; `fn error_policy(Arc<MicromegasInstance>, &Error, Arc<Context>) -> Action`; `fn resync_interval(spec: &MicromegasInstanceSpec) -> Result<Duration, String>`; `fn matching_instances(selector: &LabelSelector, candidates: Vec<Arc<MicromegasInstance>>) -> Result<Vec<Arc<MicromegasInstance>>, String>`; `fn instance_is_ready(&MicromegasInstance) -> bool`; `async fn build_client(&MicromegasInstance, &Context) -> Result<WebApiClient, BuildClientError>`; `BuildClientError { SecretNotFound(String), InvalidSpec(String), Kube(kube::Error) }`.

- [ ] **Step 1: Write the failing tests** (bottom of `instance.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crds::{MicromegasInstanceSpec, MicromegasInstanceStatus};
    use kube::core::ObjectMeta;
    use std::collections::BTreeMap;

    fn spec(resync: &str) -> MicromegasInstanceSpec {
        MicromegasInstanceSpec { url: "http://x".into(), auth: None, resync_interval: resync.into() }
    }

    #[test]
    fn resync_interval_parses_humantime() {
        assert_eq!(resync_interval(&spec("10m")).unwrap(), Duration::from_secs(600));
        assert_eq!(resync_interval(&spec("1h 30m")).unwrap(), Duration::from_secs(5400));
    }

    #[test]
    fn resync_interval_enforces_minimum() {
        assert!(resync_interval(&spec("30s")).unwrap_err().contains("at least 1m"));
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
                labels: Some(labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>()),
                ..Default::default()
            },
            spec: spec("10m"),
            status: None,
        })
    }

    #[test]
    fn matching_instances_filters_by_selector() {
        let selector = LabelSelector {
            match_labels: Some([("env".to_string(), "prod".to_string())].into_iter().collect()),
            ..Default::default()
        };
        let all = vec![instance("a", &[("env", "prod")]), instance("b", &[("env", "dev")]), instance("c", &[])];
        let matched = matching_instances(&selector, all).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].metadata.name.as_deref(), Some("a"));
    }

    #[test]
    fn empty_selector_matches_nothing() {
        // An empty selector would otherwise select every instance; Screens must opt in explicitly.
        let all = vec![instance("a", &[("env", "prod")])];
        assert!(matching_instances(&LabelSelector::default(), all).unwrap().is_empty());
    }

    #[test]
    fn instance_is_ready_reads_condition() {
        let mut inst = (*instance("a", &[])).clone();
        assert!(!instance_is_ready(&inst));
        inst.status = Some(MicromegasInstanceStatus {
            observed_generation: None,
            conditions: vec![crate::conditions::ready(true, crate::conditions::reasons::CONNECTED, "", None)],
        });
        assert!(instance_is_ready(&inst));
    }
}
```

Plus tests for `Backoff` in `mod.rs`:

```rust
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
        for _ in 0..10 { b.next("k"); }
        assert_eq!(b.next("k"), Duration::from_secs(600));
        b.reset("k");
        assert_eq!(b.next("k"), Duration::from_secs(30));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator reconcile`
Expected: compile FAIL.

- [ ] **Step 3: Implement `reconcile/mod.rs`**

```rust
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

pub async fn patch_status<K>(api: &Api<K>, name: &str, status: &impl Serialize) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + Clone + DeserializeOwned + Debug,
{
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .map(|_| ())
}
```

(Note `Backoff::next` with `1u32 << min(n,5)`: 30·1, 30·2, 30·4, 30·8, 30·16=480, 30·32=960→cap 600. Matches the test.)

- [ ] **Step 4: Implement `reconcile/instance.rs`**

```rust
use super::{Context, Error, patch_status};
use crate::auth::{OidcClientCredentials, TokenCache};
use crate::client::{ApiError, Credentials, WebApiClient};
use crate::conditions::{self, reasons};
use crate::crds::{MicromegasInstance, MicromegasInstanceSpec};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::core::Selector;
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use micromegas::tracing::prelude::*;
use std::sync::Arc;
use std::time::Duration;

const PROBE_INTERVAL: Duration = Duration::from_secs(300);
const MIN_RESYNC: Duration = Duration::from_secs(60);

pub fn resync_interval(spec: &MicromegasInstanceSpec) -> Result<Duration, String> {
    let parsed = humantime::parse_duration(&spec.resync_interval)
        .map_err(|e| format!("resyncInterval '{}' is invalid: {e}", spec.resync_interval))?;
    if parsed < MIN_RESYNC {
        return Err(format!(
            "resyncInterval '{}' must be at least 1m",
            spec.resync_interval
        ));
    }
    Ok(parsed)
}

pub fn instance_is_ready(instance: &MicromegasInstance) -> bool {
    instance
        .status
        .as_ref()
        .is_some_and(|s| conditions::is_ready(&s.conditions))
}

pub fn matching_instances(
    selector: &LabelSelector,
    candidates: Vec<Arc<MicromegasInstance>>,
) -> Result<Vec<Arc<MicromegasInstance>>, String> {
    let selector = Selector::try_from(selector.clone()).map_err(|e| e.to_string())?;
    if selector.selects_all() {
        return Ok(Vec::new());
    }
    Ok(candidates
        .into_iter()
        .filter(|inst| selector.matches(inst.labels()))
        .collect())
}

#[derive(Debug, thiserror::Error)]
pub enum BuildClientError {
    #[error("secret '{0}' not found or missing keys")]
    SecretNotFound(String),
    #[error("{0}")]
    InvalidSpec(String),
    #[error(transparent)]
    Kube(#[from] kube::Error),
}

pub async fn build_client(
    instance: &MicromegasInstance,
    ctx: &Context,
) -> Result<WebApiClient, BuildClientError> {
    let Some(auth) = &instance.spec.auth else {
        return Ok(WebApiClient::new(ctx.http.clone(), &instance.spec.url, Credentials::None));
    };
    let oidc = &auth.oidc_client_credentials;
    let namespace = instance
        .namespace()
        .ok_or_else(|| BuildClientError::InvalidSpec("instance has no namespace".into()))?;
    let secrets: Api<Secret> = Api::namespaced(ctx.client.clone(), &namespace);
    let secret = match secrets.get(&oidc.secret_ref.name).await {
        Ok(s) => s,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            return Err(BuildClientError::SecretNotFound(oidc.secret_ref.name.clone()));
        }
        Err(e) => return Err(e.into()),
    };
    let read = |key: &str| -> Result<String, BuildClientError> {
        secret
            .data
            .as_ref()
            .and_then(|d| d.get(key))
            .and_then(|bytes| String::from_utf8(bytes.0.clone()).ok())
            .ok_or_else(|| BuildClientError::SecretNotFound(format!("{}/{key}", oidc.secret_ref.name)))
    };
    let creds = OidcClientCredentials {
        issuer: oidc.issuer.clone(),
        client_id: read(&oidc.secret_ref.client_id_key)?,
        client_secret: read(&oidc.secret_ref.client_secret_key)?,
        audience: oidc.audience.clone(),
    };
    let fingerprint = creds.fingerprint();
    let uid = instance.uid().unwrap_or_default();
    let cache = {
        let mut tokens = ctx.tokens.lock().expect("token map mutex");
        match tokens.get(&uid) {
            Some((fp, cache)) if *fp == fingerprint => cache.clone(),
            _ => {
                let cache = Arc::new(TokenCache::new(ctx.http.clone(), creds));
                tokens.insert(uid, (fingerprint, cache.clone()));
                cache
            }
        }
    };
    Ok(WebApiClient::new(ctx.http.clone(), &instance.spec.url, Credentials::Oidc(cache)))
}

async fn check(instance: &MicromegasInstance, ctx: &Context) -> Result<(), (&'static str, String)> {
    resync_interval(&instance.spec).map_err(|m| (reasons::INVALID_SPEC, m))?;
    let client = build_client(instance, ctx).await.map_err(|e| match e {
        BuildClientError::SecretNotFound(_) => (reasons::SECRET_NOT_FOUND, e.to_string()),
        BuildClientError::InvalidSpec(_) => (reasons::INVALID_SPEC, e.to_string()),
        BuildClientError::Kube(_) => (reasons::UNREACHABLE, e.to_string()),
    })?;
    client.probe().await.map_err(|e| match e {
        ApiError::Unauthorized => (reasons::UNAUTHORIZED, e.to_string()),
        ApiError::Token(_) => (reasons::TOKEN_ERROR, e.to_string()),
        other => (reasons::UNREACHABLE, other.to_string()),
    })
}

pub async fn reconcile(instance: Arc<MicromegasInstance>, ctx: Arc<Context>) -> Result<Action, Error> {
    let namespace = instance.namespace().unwrap_or_default();
    let name = instance.name_any();
    let api: Api<MicromegasInstance> = Api::namespaced(ctx.client.clone(), &namespace);

    let outcome = check(&instance, &ctx).await;
    let generation = instance.metadata.generation;
    let condition = match &outcome {
        Ok(()) => conditions::ready(true, reasons::CONNECTED, "probe succeeded", generation),
        Err((reason, message)) => {
            warn!("instance {namespace}/{name} not ready: {reason}: {message}");
            conditions::ready(false, reason, message, generation)
        }
    };
    let mut status = instance.status.clone().unwrap_or_default();
    status.observed_generation = generation;
    conditions::upsert(&mut status.conditions, condition);
    patch_status(&api, &name, &status).await?;

    let key = format!("{namespace}/{name}");
    Ok(match outcome {
        Ok(()) => {
            ctx.backoff.reset(&key);
            Action::requeue(PROBE_INTERVAL)
        }
        Err(_) => Action::requeue(ctx.backoff.next(&key)),
    })
}

pub fn error_policy(instance: Arc<MicromegasInstance>, err: &Error, ctx: Arc<Context>) -> Action {
    let key = format!("{}/{}", instance.namespace().unwrap_or_default(), instance.name_any());
    warn!("instance {key} reconcile failed: {err}");
    Action::requeue(ctx.backoff.next(&key))
}
```

`Selector::matches` takes `&BTreeMap<String, String>`; `ResourceExt::labels()` returns exactly that. If `matches` is not an inherent method in kube 4.2, add `use kube::core::SelectorExt;`. If `selects_all` is not available, replace the check with `selector == Selector::default()`.

Add `pub mod reconcile;` to `src/lib.rs`.

- [ ] **Step 5: Run tests**

Run: `cd rust && cargo test -p micromegas-operator reconcile`
Expected: 7 PASS (6 in instance, 1 backoff).

- [ ] **Step 6: Clippy and commit**

Run: `cd rust && cargo clippy -p micromegas-operator -- -D warnings`

```bash
git add rust/micromegas-operator
git commit -m "Add reconcile context and MicromegasInstance controller"
```

---

### Task 8: Screen controller

**Files:**
- Create: `rust/micromegas-operator/src/reconcile/screen.rs`
- Modify: `src/reconcile/mod.rs` (add `pub mod screen;`)

**Interfaces:**
- Consumes: everything from Tasks 3–7.
- Produces: `screen::FINALIZER: &str = "micromegas.info/screen"`; `async fn reconcile(Arc<Screen>, Arc<Context>) -> Result<Action, Error>`; `fn error_policy(Arc<Screen>, &Error, Arc<Context>) -> Action`; `struct Failure { reason: &'static str, message: String }`; `fn desired_identity(screen: &Screen, cluster: &str) -> Result<Identity, Failure>` where `Identity { name, screen_type, folder_path, managed_by }`; `fn config_from_configmap(cm: Option<&ConfigMap>, cm_name: &str, key: &str) -> Result<serde_json::Value, Failure>`; `fn aggregate(outcomes: &[InstanceOutcome]) -> (bool, &'static str, String)`; `enum InstanceOutcome { Synced { status: ScreenInstanceStatus }, Failed { status: ScreenInstanceStatus, reason: &'static str, transient: bool } }`.

- [ ] **Step 1: Write the failing tests** (bottom of `screen.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crds::{ConfigFrom, ConfigMapKeyRef, ScreenSpec};
    use kube::core::ObjectMeta;
    use serde_json::json;

    fn screen(meta_name: &str, spec_name: Option<&str>, folder: &str) -> Screen {
        Screen {
            metadata: ObjectMeta { name: Some(meta_name.into()), namespace: Some("ns".into()), ..Default::default() },
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
        assert_eq!(desired_identity(&s, "c").unwrap_err().reason, reasons::INVALID_CONFIG);
    }

    fn configmap(data: &[(&str, &str)]) -> ConfigMap {
        ConfigMap {
            data: Some(data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
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
        let err = config_from_configmap(Some(&configmap(&[("k", "{not json")])), "cm", "k").unwrap_err();
        assert_eq!(err.reason, reasons::INVALID_CONFIG);
        let err = config_from_configmap(Some(&configmap(&[("k", "[1,2]")])), "cm", "k").unwrap_err();
        assert_eq!(err.reason, reasons::INVALID_CONFIG);
        let ok = config_from_configmap(Some(&configmap(&[("k", r#"{"cells":[]}"#)])), "cm", "k").unwrap();
        assert_eq!(ok, json!({"cells": []}));
    }

    fn status(name: &str) -> ScreenInstanceStatus {
        ScreenInstanceStatus { name: name.into(), namespace: "m".into(), screen_name: "s".into(), config_hash: None, last_synced_at: None, error: None }
    }

    #[test]
    fn aggregate_all_synced_is_ready() {
        let (ok, reason, _) = aggregate(&[InstanceOutcome::Synced { status: status("a") }]);
        assert!(ok);
        assert_eq!(reason, reasons::SYNCED);
    }

    #[test]
    fn aggregate_reports_first_failure() {
        let outcomes = [
            InstanceOutcome::Synced { status: status("a") },
            InstanceOutcome::Failed { status: status("b"), reason: reasons::CONFLICT, transient: false },
            InstanceOutcome::Failed { status: status("c"), reason: reasons::API_ERROR, transient: true },
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
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd rust && cargo test -p micromegas-operator screen`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/reconcile/screen.rs`:

```rust
use super::instance::{build_client, instance_is_ready, matching_instances, resync_interval};
use super::{Context, Error, patch_status};
use crate::client::ApiError;
use crate::conditions::{self, reasons};
use crate::crds::{MicromegasInstance, Screen, ScreenInstanceStatus};
use crate::plan::{self, Action as PlanAction, Desired};
use analytics_web_api::{ScreenType, validate_folder_path, validate_name};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::runtime::controller::Action;
use kube::runtime::events::{Event, EventType};
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Api, Resource, ResourceExt};
use micromegas::tracing::prelude::*;
use std::sync::Arc;
use std::time::Duration;

pub const FINALIZER: &str = "micromegas.info/screen";
const DEFAULT_REQUEUE: Duration = Duration::from_secs(600);

pub struct Failure {
    pub reason: &'static str,
    pub message: String,
}

pub struct Identity {
    pub name: String,
    pub screen_type: String,
    pub folder_path: String,
    pub managed_by: String,
}

pub enum InstanceOutcome {
    Synced { status: ScreenInstanceStatus },
    Failed { status: ScreenInstanceStatus, reason: &'static str, transient: bool },
}

pub fn desired_identity(screen: &Screen, cluster: &str) -> Result<Identity, Failure> {
    let cr_name = screen.name_any();
    let namespace = screen.namespace().unwrap_or_default();
    let name = screen.spec.name.clone().unwrap_or_else(|| cr_name.clone());
    validate_name(&name).map_err(|e| Failure {
        reason: reasons::INVALID_NAME,
        message: format!("screen name '{name}': {}", e.message),
    })?;
    validate_folder_path(&screen.spec.folder_path).map_err(|e| Failure {
        reason: reasons::INVALID_NAME,
        message: format!("folderPath '{}': {}", screen.spec.folder_path, e.message),
    })?;
    screen.spec.screen_type.parse::<ScreenType>().map_err(|e| Failure {
        reason: reasons::INVALID_CONFIG,
        message: e.to_string(),
    })?;
    Ok(Identity {
        name,
        screen_type: screen.spec.screen_type.clone(),
        folder_path: screen.spec.folder_path.clone(),
        managed_by: plan::managed_by(cluster, &namespace, &cr_name),
    })
}

pub fn config_from_configmap(
    cm: Option<&ConfigMap>,
    cm_name: &str,
    key: &str,
) -> Result<serde_json::Value, Failure> {
    let cm = cm.ok_or_else(|| Failure {
        reason: reasons::CONFIG_MAP_NOT_FOUND,
        message: format!("configmap '{cm_name}' not found"),
    })?;
    let raw = cm
        .data
        .as_ref()
        .and_then(|d| d.get(key))
        .ok_or_else(|| Failure {
            reason: reasons::CONFIG_MAP_NOT_FOUND,
            message: format!("configmap '{cm_name}' has no key '{key}'"),
        })?;
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| Failure {
        reason: reasons::INVALID_CONFIG,
        message: format!("configmap '{cm_name}' key '{key}' is not valid JSON: {e}"),
    })?;
    if !value.is_object() {
        return Err(Failure {
            reason: reasons::INVALID_CONFIG,
            message: format!("configmap '{cm_name}' key '{key}' must hold a JSON object"),
        });
    }
    Ok(value)
}

pub fn aggregate(outcomes: &[InstanceOutcome]) -> (bool, &'static str, String) {
    if outcomes.is_empty() {
        return (false, reasons::NO_MATCHING_INSTANCE, "no MicromegasInstance matches instanceSelector".into());
    }
    for outcome in outcomes {
        if let InstanceOutcome::Failed { status, reason, .. } = outcome {
            let detail = status.error.clone().unwrap_or_default();
            return (false, reason, format!("instance {}/{}: {detail}", status.namespace, status.name));
        }
    }
    (true, reasons::SYNCED, format!("synced to {} instance(s)", outcomes.len()))
}

async fn resolve_config(screen: &Screen, ctx: &Context) -> Result<serde_json::Value, Failure> {
    if let Some(config) = &screen.spec.config {
        return Ok(config.clone());
    }
    let Some(from) = &screen.spec.config_from else {
        return Err(Failure { reason: reasons::INVALID_CONFIG, message: "neither config nor configFrom is set".into() });
    };
    let reference = &from.config_map_key_ref;
    let api: Api<ConfigMap> = Api::namespaced(ctx.client.clone(), &screen.namespace().unwrap_or_default());
    let cm = match api.get(&reference.name).await {
        Ok(cm) => Some(cm),
        Err(kube::Error::Api(e)) if e.code == 404 => None,
        Err(e) => return Err(Failure { reason: reasons::API_ERROR, message: e.to_string() }),
    };
    config_from_configmap(cm.as_ref(), &reference.name, &reference.key)
}

fn instance_status(instance: &MicromegasInstance, screen_name: &str) -> ScreenInstanceStatus {
    ScreenInstanceStatus {
        name: instance.name_any(),
        namespace: instance.namespace().unwrap_or_default(),
        screen_name: screen_name.to_string(),
        config_hash: None,
        last_synced_at: None,
        error: None,
    }
}

async fn sync_one(
    screen: &Screen,
    desired: &Desired,
    instance: &MicromegasInstance,
    ctx: &Context,
) -> InstanceOutcome {
    let mut status = instance_status(instance, &desired.name);
    let fail = |mut status: ScreenInstanceStatus, reason: &'static str, message: String, transient: bool| {
        status.error = Some(message);
        InstanceOutcome::Failed { status, reason, transient }
    };
    if !instance_is_ready(instance) {
        return fail(status, reasons::INSTANCE_NOT_READY, "instance is not Ready".into(), false);
    }
    let client = match build_client(instance, ctx).await {
        Ok(c) => c,
        Err(e) => return fail(status, reasons::INSTANCE_NOT_READY, e.to_string(), true),
    };
    let current = match client.get_screen(&desired.name).await {
        Ok(c) => c,
        Err(e) => return fail(status, reasons::API_ERROR, e.to_string(), e.is_transient()),
    };
    let action = plan::plan(desired, current.as_ref());
    let result = match &action {
        PlanAction::Create(req) => client.create_screen(req).await.map(|_| "Created"),
        PlanAction::Update(req) => client.update_screen(&desired.name, req).await.map(|_| "Updated"),
        PlanAction::NoOp => Ok("InSync"),
        PlanAction::Conflict(message) => {
            publish(ctx, screen, EventType::Warning, "Conflict", message).await;
            return fail(status, reasons::CONFLICT, message.clone(), false);
        }
    };
    match result {
        Ok(verb) => {
            if verb != "InSync" {
                publish(ctx, screen, EventType::Normal, verb, &format!("{verb} screen '{}' on {}", desired.name, instance.name_any())).await;
            }
            status.config_hash = Some(plan::config_hash(&desired.config));
            status.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
            InstanceOutcome::Synced { status }
        }
        Err(ApiError::BadRequest(e)) => fail(status, reasons::INVALID_CONFIG, format!("{}: {}", e.code, e.message), false),
        Err(ApiError::Unauthorized) => fail(status, reasons::INSTANCE_NOT_READY, "unauthorized".into(), true),
        Err(e) => fail(status, reasons::API_ERROR, e.to_string(), e.is_transient()),
    }
}

async fn publish(ctx: &Context, screen: &Screen, type_: EventType, action: &str, note: &str) {
    let event = Event {
        type_,
        reason: action.to_string(),
        note: Some(note.to_string()),
        action: action.to_string(),
        secondary: None,
    };
    if let Err(e) = ctx.recorder.publish(&event, &screen.object_ref(&())).await {
        debug!("event publish failed: {e}");
    }
}

async fn write_status(
    api: &Api<Screen>,
    screen: &Screen,
    ok: bool,
    reason: &str,
    message: &str,
    instances: Vec<ScreenInstanceStatus>,
) -> Result<(), Error> {
    let generation = screen.metadata.generation;
    let mut status = screen.status.clone().unwrap_or_default();
    status.observed_generation = generation;
    status.instances = instances;
    conditions::upsert(&mut status.conditions, conditions::ready(ok, reason, message, generation));
    patch_status(api, &screen.name_any(), &status).await?;
    Ok(())
}

async fn apply(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let namespace = screen.namespace().unwrap_or_default();
    let api: Api<Screen> = Api::namespaced(ctx.client.clone(), &namespace);
    let key = format!("{namespace}/{}", screen.name_any());

    let identity = match desired_identity(&screen, &ctx.cluster_name) {
        Ok(id) => id,
        Err(f) => {
            write_status(&api, &screen, false, f.reason, &f.message, vec![]).await?;
            return Ok(Action::await_change());
        }
    };
    let config = match resolve_config(&screen, &ctx).await {
        Ok(c) => c,
        Err(f) => {
            write_status(&api, &screen, false, f.reason, &f.message, vec![]).await?;
            return Ok(Action::await_change());
        }
    };
    let desired = Desired {
        name: identity.name,
        screen_type: identity.screen_type,
        folder_path: identity.folder_path,
        config,
        managed_by: identity.managed_by,
    };

    let instances = matching_instances(&screen.spec.instance_selector, ctx.instances.state())
        .map_err(Error::Transient)?;
    let mut outcomes = Vec::with_capacity(instances.len());
    let mut requeue = DEFAULT_REQUEUE;
    for instance in &instances {
        if let Ok(interval) = resync_interval(&instance.spec) {
            requeue = requeue.min(interval);
        }
        outcomes.push(sync_one(&screen, &desired, instance, &ctx).await);
    }

    let (ok, reason, message) = aggregate(&outcomes);
    let statuses = outcomes
        .iter()
        .map(|o| match o {
            InstanceOutcome::Synced { status } | InstanceOutcome::Failed { status, .. } => status.clone(),
        })
        .collect();
    write_status(&api, &screen, ok, reason, &message, statuses).await?;

    let transient = outcomes
        .iter()
        .any(|o| matches!(o, InstanceOutcome::Failed { transient: true, .. }));
    if transient {
        return Ok(Action::requeue(ctx.backoff.next(&key)));
    }
    ctx.backoff.reset(&key);
    if instances.is_empty() {
        return Ok(Action::await_change());
    }
    Ok(Action::requeue(requeue))
}

async fn cleanup(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let identity = match desired_identity(&screen, &ctx.cluster_name) {
        Ok(id) => id,
        // Nothing valid could ever have been written under an invalid identity.
        Err(_) => return Ok(Action::await_change()),
    };
    let instances = matching_instances(&screen.spec.instance_selector, ctx.instances.state())
        .map_err(Error::Transient)?;
    for instance in instances {
        let client = build_client(&instance, &ctx)
            .await
            .map_err(|e| Error::Transient(e.to_string()))?;
        match client.get_screen(&identity.name).await {
            Ok(Some(current)) if current.managed_by.as_deref() == Some(identity.managed_by.as_str()) => {
                client
                    .delete_screen(&identity.name)
                    .await
                    .map_err(|e| Error::Transient(e.to_string()))?;
                info!("deleted screen '{}' from {}", identity.name, instance.name_any());
                publish(&ctx, &screen, EventType::Normal, "Deleted", &format!("deleted screen '{}' on {}", identity.name, instance.name_any())).await;
            }
            Ok(_) => {}
            Err(e) => return Err(Error::Transient(e.to_string())),
        }
    }
    Ok(Action::await_change())
}

pub async fn reconcile(screen: Arc<Screen>, ctx: Arc<Context>) -> Result<Action, Error> {
    let namespace = screen.namespace().unwrap_or_default();
    let api: Api<Screen> = Api::namespaced(ctx.client.clone(), &namespace);
    finalizer(&api, FINALIZER, screen, |event| async {
        match event {
            FinalizerEvent::Apply(s) => apply(s, ctx.clone()).await,
            FinalizerEvent::Cleanup(s) => cleanup(s, ctx.clone()).await,
        }
    })
    .await
    .map_err(|e| Error::Finalizer(e.to_string()))
}

pub fn error_policy(screen: Arc<Screen>, err: &Error, ctx: Arc<Context>) -> Action {
    let key = format!("{}/{}", screen.namespace().unwrap_or_default(), screen.name_any());
    warn!("screen {key} reconcile failed: {err}");
    Action::requeue(ctx.backoff.next(&key))
}
```

Add `pub mod screen;` to `src/reconcile/mod.rs`. The `finalizer` closure captures `ctx` by reference across an `async` block; if the borrow checker objects, clone `ctx` before the call (`let ctx2 = ctx.clone();`) and move it into the closure.

- [ ] **Step 4: Run tests**

Run: `cd rust && cargo test -p micromegas-operator screen`
Expected: 9 PASS.

- [ ] **Step 5: Clippy and commit**

Run: `cd rust && cargo clippy -p micromegas-operator -- -D warnings`

```bash
git add rust/micromegas-operator
git commit -m "Add Screen controller with finalizer and per-instance sync"
```

---

### Task 9: Binary wiring, health probes, telemetry

**Files:**
- Create: `rust/micromegas-operator/src/health.rs`
- Modify: `rust/micromegas-operator/src/main.rs` (replace placeholder), `src/lib.rs` (add `pub mod health;`)

**Interfaces:**
- Consumes: `reconcile::{Context, Backoff, instance, screen}`, `crds::*`.
- Produces: binary `micromegas-operator` with flags `--cluster-name` (env `MICROMEGAS_OPERATOR_CLUSTER_NAME`, required), `--watch-namespace` (env `MICROMEGAS_OPERATOR_WATCH_NAMESPACE`, optional), `--health-listen` (env `MICROMEGAS_OPERATOR_HEALTH_LISTEN`, default `0.0.0.0:8080`); `health::serve(addr: SocketAddr) -> impl Future<Output = anyhow::Result<()>>` exposing `GET /healthz` and `GET /readyz` returning `200 ok`.

- [ ] **Step 1: health.rs**

```rust
use axum::{Router, routing::get};
use std::net::SocketAddr;

pub async fn serve(addr: SocketAddr) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

Add `pub mod health;` to `src/lib.rs`.

- [ ] **Step 2: main.rs**

```rust
//! Kubernetes operator: reconciles Screen custom resources into analytics-web-srv screens.
//!
//! Telemetry is configured by `micromegas_main` from the usual env vars
//! (`MICROMEGAS_TELEMETRY_URL`, `MICROMEGAS_INGESTION_API_KEY`, ...), so the
//! operator can report into the Micromegas it manages.

use clap::Parser;
use futures::StreamExt;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use kube::runtime::controller::Controller;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::reflector::ObjectRef;
use kube::runtime::watcher;
use kube::{Api, Client, ResourceExt};
use micromegas::micromegas_main;
use micromegas::tracing::prelude::*;
use micromegas_operator::crds::{MicromegasInstance, Screen};
use micromegas_operator::reconcile::instance::matching_instances;
use micromegas_operator::reconcile::{Backoff, Context, FIELD_MANAGER, instance, screen};
use micromegas_operator::health;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[derive(Parser, Debug)]
#[clap(name = "micromegas-operator", about = "Kubernetes operator for Micromegas screens", version)]
struct Cli {
    /// Stable name of this cluster; part of every managed screen's managed_by marker.
    #[clap(long, env = "MICROMEGAS_OPERATOR_CLUSTER_NAME")]
    cluster_name: String,

    /// Restrict watches to one namespace. Default: all namespaces.
    #[clap(long, env = "MICROMEGAS_OPERATOR_WATCH_NAMESPACE")]
    watch_namespace: Option<String>,

    #[clap(long, env = "MICROMEGAS_OPERATOR_HEALTH_LISTEN", default_value = "0.0.0.0:8080")]
    health_listen: SocketAddr,
}

fn api<K>(client: &Client, namespace: Option<&str>) -> Api<K>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope, DynamicType = ()>,
{
    match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    }
}

#[micromegas_main(interop_max_level = "info")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let client = Client::try_default().await?;
    let ns = cli.watch_namespace.as_deref();

    let instances_api: Api<MicromegasInstance> = api(&client, ns);
    let screens_api: Api<Screen> = api(&client, ns);
    let secrets_api: Api<Secret> = api(&client, ns);
    let configmaps_api: Api<ConfigMap> = api(&client, ns);

    let instance_ctrl = Controller::new(instances_api.clone(), watcher::Config::default());
    let instances_store = instance_ctrl.store();
    let screen_ctrl = Controller::new(screens_api, watcher::Config::default());
    let screens_store = screen_ctrl.store();

    let ctx = Arc::new(Context {
        client: client.clone(),
        http: reqwest::Client::builder().build()?,
        cluster_name: cli.cluster_name.clone(),
        instances: instances_store.clone(),
        tokens: Mutex::new(Default::default()),
        recorder: Recorder::new(
            client.clone(),
            Reporter { controller: FIELD_MANAGER.into(), instance: std::env::var("HOSTNAME").ok() },
        ),
        backoff: Backoff::default(),
    });

    // Instance changes re-enqueue every Screen whose selector matches it.
    let screens_for_instances = screens_store.clone();
    let screens_for_configmaps = screens_store.clone();
    let screen_ctrl = screen_ctrl
        .watches(instances_api, watcher::Config::default(), move |inst: MicromegasInstance| {
            let inst = Arc::new(inst);
            screens_for_instances
                .state()
                .into_iter()
                .filter(|s| {
                    matching_instances(&s.spec.instance_selector, vec![inst.clone()])
                        .map(|m| !m.is_empty())
                        .unwrap_or(false)
                })
                .map(|s| ObjectRef::from_obj(&*s))
                .collect::<Vec<_>>()
        })
        .watches(configmaps_api, watcher::Config::default(), move |cm: ConfigMap| {
            screens_for_configmaps
                .state()
                .into_iter()
                .filter(|s| {
                    s.namespace() == cm.namespace()
                        && s.spec
                            .config_from
                            .as_ref()
                            .is_some_and(|c| c.config_map_key_ref.name == cm.name_any())
                })
                .map(|s| ObjectRef::from_obj(&*s))
                .collect::<Vec<_>>()
        })
        .shutdown_on_signal();

    // Secret changes re-enqueue the instances that reference them.
    let instances_for_secrets = instances_store.clone();
    let instance_ctrl = instance_ctrl
        .watches(secrets_api, watcher::Config::default(), move |secret: Secret| {
            instances_for_secrets
                .state()
                .into_iter()
                .filter(|i| {
                    i.namespace() == secret.namespace()
                        && i.spec.auth.as_ref().is_some_and(|a| {
                            a.oidc_client_credentials.secret_ref.name == secret.name_any()
                        })
                })
                .map(|i| ObjectRef::from_obj(&*i))
                .collect::<Vec<_>>()
        })
        .shutdown_on_signal();

    info!("micromegas-operator starting, cluster_name={}", cli.cluster_name);

    let instance_loop = instance_ctrl
        .run(instance::reconcile, instance::error_policy, ctx.clone())
        .for_each(|res| async move {
            match res {
                Ok((obj, _)) => debug!("reconciled instance {obj}"),
                Err(e) => warn!("instance controller error: {e}"),
            }
        });
    let screen_loop = screen_ctrl
        .run(screen::reconcile, screen::error_policy, ctx.clone())
        .for_each(|res| async move {
            match res {
                Ok((obj, _)) => debug!("reconciled screen {obj}"),
                Err(e) => warn!("screen controller error: {e}"),
            }
        });

    tokio::select! {
        _ = instance_loop => {},
        _ = screen_loop => {},
        r = health::serve(cli.health_listen) => { r?; },
    }
    Ok(())
}
```

Notes for the implementer: `Controller::watches` with a store-driven mapper is the kube-rs idiom for "watch an unrelated type and fan out"; `Store::state()` returns `Vec<Arc<K>>`. The `Api` scope bound on `api()` uses `k8s_openapi::NamespaceResourceScope`; if the compiler wants a different bound spelling, follow the compiler. `tokio::select!` ends the process when either controller stream finishes (they finish on SIGTERM via `shutdown_on_signal`).

- [ ] **Step 3: Build, clippy, help**

Run:
```bash
cd rust && cargo build -p micromegas-operator && cargo clippy -p micromegas-operator -- -D warnings
cargo run -p micromegas-operator -- --help
```
Expected: help lists `--cluster-name`, `--watch-namespace`, `--health-listen`. Running without `--cluster-name` exits with clap's "required" error.

- [ ] **Step 4: Smoke against kind (manual, no cluster needed if kind is absent — skip and say so)**

If `kind` and `kubectl` are installed:
```bash
kind create cluster --name mm-smoke
kubectl apply -f ../charts/micromegas-operator/crds/
MICROMEGAS_OPERATOR_CLUSTER_NAME=smoke cargo run -p micromegas-operator -- --health-listen 127.0.0.1:8080 &
sleep 5; curl -s http://127.0.0.1:8080/healthz; echo
kubectl apply -f - <<'EOF'
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata: { name: nowhere, namespace: default, labels: { env: smoke } }
spec: { url: "http://127.0.0.1:9" }
EOF
sleep 5; kubectl get mmi nowhere -o jsonpath='{.status.conditions[0].reason}'; echo
kill %1; kind delete cluster --name mm-smoke
```
Expected: `ok`, then `Unreachable`.

- [ ] **Step 5: Commit**

```bash
git add rust/micromegas-operator
git commit -m "Wire micromegas-operator controllers, health probes, and CLI"
```

---

### Task 10: Helm chart

**Files:**
- Create: `charts/micromegas-operator/Chart.yaml`, `values.yaml`, `README.md`, `templates/_helpers.tpl`, `templates/serviceaccount.yaml`, `templates/rbac.yaml`, `templates/deployment.yaml`, `examples/instance.yaml`, `examples/screen-inline.yaml`, `examples/screen-configmap.yaml`
- Existing: `charts/micromegas-operator/crds/*.yaml` (Task 2)

- [ ] **Step 1: Chart.yaml and values.yaml**

`Chart.yaml`:
```yaml
apiVersion: v2
name: micromegas-operator
description: Kubernetes operator that manages Micromegas screens from Screen custom resources
type: application
version: 0.1.0
appVersion: "0.32.0"
home: https://micromegas.info/
sources:
  - https://github.com/madesroches/micromegas
```

`values.yaml`:
```yaml
image:
  repository: marcantoinedesroches/micromegas-operator
  tag: ""            # defaults to appVersion
  pullPolicy: IfNotPresent

# Required. Stable identifier of this cluster; becomes part of every managed
# screen's managed_by marker (k8s://<clusterName>/<namespace>/<name>).
clusterName: ""

# Empty watches all namespaces (ClusterRole). Set to restrict to one namespace (Role).
watchNamespace: ""

healthPort: 8080

# Self-telemetry into a Micromegas ingestion server. Leave url empty to disable.
telemetry:
  url: ""
  apiKeySecret:
    name: ""
    key: api-key

resources:
  requests: { cpu: 50m, memory: 64Mi }
  limits: { memory: 256Mi }

nodeSelector: {}
tolerations: []
affinity: {}
podAnnotations: {}
```

- [ ] **Step 2: Templates**

`templates/_helpers.tpl`:
```yaml
{{- define "micromegas-operator.name" -}}
{{ .Chart.Name }}
{{- end }}
{{- define "micromegas-operator.fullname" -}}
{{- if contains .Chart.Name .Release.Name }}{{ .Release.Name }}{{ else }}{{ printf "%s-%s" .Release.Name .Chart.Name }}{{ end }}
{{- end }}
{{- define "micromegas-operator.labels" -}}
app.kubernetes.io/name: {{ include "micromegas-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}
{{- define "micromegas-operator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "micromegas-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
```

`templates/serviceaccount.yaml`:
```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "micromegas-operator.fullname" . }}
  labels: {{- include "micromegas-operator.labels" . | nindent 4 }}
```

`templates/rbac.yaml`:
```yaml
{{- $kind := ternary "Role" "ClusterRole" (ne .Values.watchNamespace "") }}
{{- $bindingKind := ternary "RoleBinding" "ClusterRoleBinding" (ne .Values.watchNamespace "") }}
apiVersion: rbac.authorization.k8s.io/v1
kind: {{ $kind }}
metadata:
  name: {{ include "micromegas-operator.fullname" . }}
  {{- if ne .Values.watchNamespace "" }}
  namespace: {{ .Values.watchNamespace }}
  {{- end }}
  labels: {{- include "micromegas-operator.labels" . | nindent 4 }}
rules:
  - apiGroups: ["micromegas.info"]
    resources: ["screens", "micromegasinstances"]
    verbs: ["get", "list", "watch", "update", "patch"]
  - apiGroups: ["micromegas.info"]
    resources: ["screens/status", "micromegasinstances/status"]
    verbs: ["get", "update", "patch"]
  - apiGroups: ["micromegas.info"]
    resources: ["screens/finalizers"]
    verbs: ["update"]
  - apiGroups: [""]
    resources: ["configmaps", "secrets"]
    verbs: ["get", "list", "watch"]
  - apiGroups: ["", "events.k8s.io"]
    resources: ["events"]
    verbs: ["create", "patch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: {{ $bindingKind }}
metadata:
  name: {{ include "micromegas-operator.fullname" . }}
  {{- if ne .Values.watchNamespace "" }}
  namespace: {{ .Values.watchNamespace }}
  {{- end }}
  labels: {{- include "micromegas-operator.labels" . | nindent 4 }}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: {{ $kind }}
  name: {{ include "micromegas-operator.fullname" . }}
subjects:
  - kind: ServiceAccount
    name: {{ include "micromegas-operator.fullname" . }}
    namespace: {{ .Release.Namespace }}
```

`templates/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "micromegas-operator.fullname" . }}
  labels: {{- include "micromegas-operator.labels" . | nindent 4 }}
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels: {{- include "micromegas-operator.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels: {{- include "micromegas-operator.selectorLabels" . | nindent 8 }}
      {{- with .Values.podAnnotations }}
      annotations: {{- toYaml . | nindent 8 }}
      {{- end }}
    spec:
      serviceAccountName: {{ include "micromegas-operator.fullname" . }}
      containers:
        - name: operator
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}"
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          env:
            - name: MICROMEGAS_OPERATOR_CLUSTER_NAME
              value: {{ required "clusterName is required" .Values.clusterName | quote }}
            {{- if ne .Values.watchNamespace "" }}
            - name: MICROMEGAS_OPERATOR_WATCH_NAMESPACE
              value: {{ .Values.watchNamespace | quote }}
            {{- end }}
            - name: MICROMEGAS_OPERATOR_HEALTH_LISTEN
              value: "0.0.0.0:{{ .Values.healthPort }}"
            {{- if .Values.telemetry.url }}
            - name: MICROMEGAS_TELEMETRY_URL
              value: {{ .Values.telemetry.url | quote }}
            {{- if .Values.telemetry.apiKeySecret.name }}
            - name: MICROMEGAS_INGESTION_API_KEY
              valueFrom:
                secretKeyRef:
                  name: {{ .Values.telemetry.apiKeySecret.name }}
                  key: {{ .Values.telemetry.apiKeySecret.key }}
            {{- end }}
            {{- end }}
          ports:
            - name: health
              containerPort: {{ .Values.healthPort }}
          livenessProbe:
            httpGet: { path: /healthz, port: health }
          readinessProbe:
            httpGet: { path: /readyz, port: health }
          resources: {{- toYaml .Values.resources | nindent 12 }}
      {{- with .Values.nodeSelector }}
      nodeSelector: {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.tolerations }}
      tolerations: {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.affinity }}
      affinity: {{- toYaml . | nindent 8 }}
      {{- end }}
```

- [ ] **Step 3: Examples**

`examples/instance.yaml`:
```yaml
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata:
  name: prod
  namespace: micromegas
  labels:
    micromegas.info/env: prod
spec:
  url: https://micromegas.example.com
  auth:
    oidcClientCredentials:
      issuer: https://idp.example.com/realms/example
      secretRef:
        name: micromegas-operator-oidc
  resyncInterval: 10m
```

`examples/screen-inline.yaml`:
```yaml
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata:
  name: service-overview
  namespace: my-service
spec:
  instanceSelector:
    matchLabels:
      micromegas.info/env: prod
  folderPath: my-service/prod
  config:
    timeRangeFrom: now-1h
    timeRangeTo: now
    cells: []
```

`examples/screen-configmap.yaml`:
```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: my-service-screens
  namespace: my-service
data:
  errors.json: |
    { "timeRangeFrom": "now-6h", "timeRangeTo": "now", "cells": [] }
---
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata:
  name: service-errors
  namespace: my-service
spec:
  instanceSelector:
    matchLabels:
      micromegas.info/env: prod
  folderPath: my-service/prod
  configFrom:
    configMapKeyRef:
      name: my-service-screens
      key: errors.json
```

- [ ] **Step 4: README.md** (chart-level; the full docs live in mkdocs, Task 13)

```markdown
# micromegas-operator

Installs the CRDs, RBAC, and Deployment for the Micromegas Kubernetes operator.

    helm install micromegas-operator ./charts/micromegas-operator \
      --namespace micromegas-system --create-namespace \
      --set clusterName=prod-eu

Helm installs `crds/` on first install only. On upgrade, apply them by hand:

    kubectl apply -f charts/micromegas-operator/crds/

See `examples/` for a `MicromegasInstance` and two `Screen` shapes, and
https://micromegas.info/docs/admin/kubernetes-operator/ for the full reference.
```

- [ ] **Step 5: Lint and render**

Run (install Helm with `brew install helm` if missing):
```bash
helm lint charts/micromegas-operator --set clusterName=x
helm template t charts/micromegas-operator --set clusterName=x | grep -c "^kind:"
helm template t charts/micromegas-operator --set clusterName=x --set watchNamespace=team | grep "^kind: Role"
helm template t charts/micromegas-operator 2>&1 | grep "clusterName is required"
```
Expected: lint passes; 4 kinds rendered (ServiceAccount, ClusterRole, ClusterRoleBinding, Deployment); Role variant when `watchNamespace` set; missing clusterName fails with the message.

- [ ] **Step 6: Commit**

```bash
git add charts/micromegas-operator
git commit -m "Add micromegas-operator Helm chart"
```

---

### Task 11: Container image

**Files:**
- Create: `docker/operator.Dockerfile`
- Modify: `build/build_docker_images.py:32-41`, `docker/README.md` (images table)

- [ ] **Step 1: Dockerfile** (copy of `docker/flight-sql.Dockerfile` with the binary name swapped and the health port exposed)

```dockerfile
# Multi-stage build for micromegas-operator
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS builder

ARG TARGETARCH

RUN if [ "$TARGETARCH" = "arm64" ]; then \
      apt-get update && \
      apt-get install -y --no-install-recommends \
        g++-aarch64-linux-gnu libc6-dev-arm64-cross && \
      rm -rf /var/lib/apt/lists/*; \
    fi

WORKDIR /build
COPY rust/ ./rust/

WORKDIR /build/rust
RUN if [ "$TARGETARCH" = "arm64" ]; then \
      rustup target add aarch64-unknown-linux-gnu && \
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
      cargo build --release --target aarch64-unknown-linux-gnu --bin micromegas-operator; \
    else \
      cargo build --release --bin micromegas-operator; \
    fi

RUN if [ "$TARGETARCH" = "arm64" ]; then ARCH_PATH="aarch64-unknown-linux-gnu/"; else ARCH_PATH=""; fi && \
    cp /build/rust/target/${ARCH_PATH}release/micromegas-operator /build/micromegas-operator

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/micromegas-operator /usr/local/bin/

EXPOSE 8080
ENTRYPOINT ["micromegas-operator"]
```

- [ ] **Step 2: Register the image**

In `build/build_docker_images.py` `SERVICES`, after the `"monolith"` entry:
```python
    "operator": ("operator.Dockerfile", "Kubernetes operator for screens"),
```
In `docker/README.md` images table, add a row:
```
| `operator.Dockerfile` | `marcantoinedesroches/micromegas-operator` | Kubernetes operator for screens |
```
and change "Eight services are published" to "Nine services are published".

- [ ] **Step 3: Build locally**

Run: `python3 build/build_docker_images.py operator`
Expected: image builds. Then `docker run --rm marcantoinedesroches/micromegas-operator:latest --help` prints the CLI help. If the build script tags differently, use `docker images | grep operator` to find the tag.

- [ ] **Step 4: Commit**

```bash
git add docker/operator.Dockerfile build/build_docker_images.py docker/README.md
git commit -m "Add micromegas-operator container image"
```

---

### Task 12: End-to-end script (manual)

**Files:**
- Create: `local_test_env/ai_scripts/operator_e2e.py`

The operator runs out-of-cluster with `cargo run` against a kind cluster, so no image load or pod-to-host networking is needed. The monolith runs on the host with auth disabled; the instance CR has no `auth`.

- [ ] **Step 1: Write the script**

```python
#!/usr/bin/env python3
"""End-to-end check of micromegas-operator against a kind cluster and a local monolith.

Prerequisites: docker, kind, kubectl, cargo. Run from anywhere:
    cd python/micromegas && poetry run python ../../local_test_env/ai_scripts/operator_e2e.py

Steps: start monolith (--disable-auth) -> kind cluster -> CRDs -> operator (cargo run)
-> MicromegasInstance Ready -> inline + ConfigMap Screens appear on the server with the
k8s:// managed_by -> implicit folder listed -> UI-style edit is reverted on resync
-> unmanaged screen with the same name yields Conflict -> deleting the CR deletes the screen.
"""

import json
import subprocess
import sys
import time
from pathlib import Path

import requests

REPO = Path(__file__).resolve().parents[2]
CLUSTER = "micromegas-operator-e2e"
WEB = "http://127.0.0.1:3000"
API = f"{WEB}/api"
CLUSTER_NAME = "e2e"


def run(cmd, check=True, input=None, capture=False):
    print(f"$ {cmd}")
    return subprocess.run(cmd, shell=True, check=check, input=input, text=True,
                          capture_output=capture)


def kubectl_apply(manifest):
    run("kubectl apply -f -", input=manifest)


def wait_for(description, predicate, timeout=120, interval=2):
    print(f"... waiting for {description}")
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if predicate():
                print(f"ok: {description}")
                return
        except Exception as e:  # noqa: BLE001 - polling
            last = e
        time.sleep(interval)
    raise SystemExit(f"TIMEOUT waiting for {description}")


def screen(name):
    r = requests.get(f"{API}/screens/{name}", timeout=5)
    return r.json() if r.status_code == 200 else None


def ready_reason(kind, name, ns="default"):
    out = run(f"kubectl -n {ns} get {kind} {name} -o json", capture=True).stdout
    conds = json.loads(out).get("status", {}).get("conditions", [])
    ready = [c for c in conds if c["type"] == "Ready"]
    return (ready[0]["status"], ready[0]["reason"]) if ready else (None, None)


INSTANCE = f"""
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata: {{ name: local, namespace: default, labels: {{ env: e2e }} }}
spec: {{ url: "{WEB}", resyncInterval: "1m" }}
"""

SCREEN_INLINE = """
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata: { name: e2e-inline, namespace: default }
spec:
  instanceSelector: { matchLabels: { env: e2e } }
  folderPath: e2e/nested
  config: { timeRangeFrom: now-1h, timeRangeTo: now, cells: [] }
"""

SCREEN_CM = """
apiVersion: v1
kind: ConfigMap
metadata: { name: e2e-screens, namespace: default }
data:
  cm.json: '{"timeRangeFrom":"now-6h","timeRangeTo":"now","cells":[]}'
---
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata: { name: e2e-from-cm, namespace: default }
spec:
  instanceSelector: { matchLabels: { env: e2e } }
  folderPath: e2e
  configFrom: { configMapKeyRef: { name: e2e-screens, key: cm.json } }
"""

SCREEN_CONFLICT = """
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata: { name: e2e-conflict, namespace: default }
spec:
  instanceSelector: { matchLabels: { env: e2e } }
  config: { cells: [] }
"""


def main():
    operator = None
    try:
        run(f"python3 {REPO}/local_test_env/ai_scripts/start_services.py --monolith")
        wait_for("monolith web API", lambda: requests.get(f"{API}/screens", timeout=2).ok)

        run(f"kind delete cluster --name {CLUSTER}", check=False)
        run(f"kind create cluster --name {CLUSTER}")
        run(f"kubectl apply -f {REPO}/charts/micromegas-operator/crds/")

        operator = subprocess.Popen(
            ["cargo", "run", "-p", "micromegas-operator", "--", "--cluster-name", CLUSTER_NAME,
             "--health-listen", "127.0.0.1:18080"],
            cwd=REPO / "rust",
        )
        wait_for("operator health", lambda: requests.get("http://127.0.0.1:18080/healthz", timeout=1).ok, timeout=600)

        kubectl_apply(INSTANCE)
        wait_for("instance Ready", lambda: ready_reason("mmi", "local") == ("True", "Connected"))

        kubectl_apply(SCREEN_INLINE)
        kubectl_apply(SCREEN_CM)
        expected_owner = f"k8s://{CLUSTER_NAME}/default/e2e-inline"
        wait_for("inline screen on server",
                 lambda: (screen("e2e-inline") or {}).get("managed_by") == expected_owner)
        wait_for("configmap screen on server",
                 lambda: (screen("e2e-from-cm") or {}).get("config", {}).get("timeRangeFrom") == "now-6h")
        wait_for("Screen CRs Ready",
                 lambda: ready_reason("screen", "e2e-inline") == ("True", "Synced")
                 and ready_reason("screen", "e2e-from-cm") == ("True", "Synced"))

        folders = {f["path"] for f in requests.get(f"{API}/folders", timeout=5).json()}
        assert "e2e/nested" in folders, folders
        print("ok: implicit folder e2e/nested listed")

        requests.put(f"{API}/screens/e2e-inline", json={"config": {"cells": [{"id": "x"}]}}, timeout=5).raise_for_status()
        wait_for("UI edit reverted on resync",
                 lambda: screen("e2e-inline")["config"]["cells"] == [], timeout=150)

        requests.post(f"{API}/screens", json={"name": "e2e-conflict", "screen_type": "notebook",
                                              "config": {"cells": []}}, timeout=5).raise_for_status()
        kubectl_apply(SCREEN_CONFLICT)
        wait_for("conflict reported", lambda: ready_reason("screen", "e2e-conflict") == ("False", "Conflict"))
        assert screen("e2e-conflict")["managed_by"] is None, "operator must not touch unmanaged screen"

        run("kubectl -n default delete screen e2e-inline")
        wait_for("screen deleted from server", lambda: screen("e2e-inline") is None)
        assert screen("e2e-from-cm") is not None

        print("\nE2E PASSED")
    finally:
        if operator:
            operator.terminate()
            operator.wait(timeout=30)
        run(f"kind delete cluster --name {CLUSTER}", check=False)
        run(f"python3 {REPO}/local_test_env/ai_scripts/stop_services.py", check=False)


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Run it**

Run: `cd python/micromegas && poetry run python ../../local_test_env/ai_scripts/operator_e2e.py`
Expected: every `ok:` line and `E2E PASSED`. If the monolith's web API is not on port 3000, check `local_test_env/ai_scripts/start_services.py --monolith` output and adjust `WEB`. If a step fails, fix the operator (not the script's expectations) and re-run.

- [ ] **Step 3: Commit**

```bash
git add local_test_env/ai_scripts/operator_e2e.py
git commit -m "Add manual end-to-end script for micromegas-operator"
```

---

### Task 13: Documentation and changelog

**Files:**
- Create: `mkdocs/docs/admin/kubernetes-operator.md`
- Modify: `mkdocs/mkdocs.yml:153-154` (nav), `mkdocs/docs/web-app/notebooks/screens-as-code.md` (cross-link in Overview), `CHANGELOG.md` (Unreleased)

- [ ] **Step 1: Write the admin page**

`mkdocs/docs/admin/kubernetes-operator.md`:

```markdown
# Kubernetes Operator

`micromegas-operator` keeps screens on an analytics web server in sync with `Screen` custom
resources in a Kubernetes cluster. A service ships its notebook dashboards in its own Helm chart,
and the operator creates, updates, and deletes them on the server.

## Install

    helm install micromegas-operator ./charts/micromegas-operator \
      --namespace micromegas-system --create-namespace \
      --set clusterName=prod-eu

`clusterName` is required. It becomes part of every managed screen's `managed_by` marker,
`k8s://<clusterName>/<namespace>/<name>`, which the web app shows as "managed by source control".

Helm installs the CRDs in `crds/` on first install only. When upgrading, apply them first:

    kubectl apply -f charts/micromegas-operator/crds/

## Authenticating the operator

The web server accepts OIDC bearer tokens. Create a client in your identity provider that supports
the client-credentials grant, store its credentials in a Secret, and list the issuer in the web
server's `MICROMEGAS_OIDC_CONFIG` (see [Analytics Web App Deployment](web-app.md)).

    kubectl -n micromegas create secret generic micromegas-operator-oidc \
      --from-literal=client_id=micromegas-operator \
      --from-literal=client_secret=...

## MicromegasInstance

One per web server the operator should write to.

    apiVersion: micromegas.info/v1alpha1
    kind: MicromegasInstance
    metadata:
      name: prod
      namespace: micromegas
      labels:
        micromegas.info/env: prod
    spec:
      url: https://micromegas.example.com          # include MICROMEGAS_BASE_PATH if set
      auth:
        oidcClientCredentials:
          issuer: https://idp.example.com/realms/example
          audience: micromegas                     # optional
          secretRef:
            name: micromegas-operator-oidc
            clientIdKey: client_id                 # default
            clientSecretKey: client_secret         # default
      resyncInterval: 10m                          # default 10m, minimum 1m

Omitting `auth` sends no token. That only works against a server started with `--disable-auth`
and is meant for local development.

The `Ready` condition reports `Connected`, or one of `SecretNotFound`, `TokenError`, `Unreachable`,
`Unauthorized`, `InvalidSpec`.

## Screen

    apiVersion: micromegas.info/v1alpha1
    kind: Screen
    metadata:
      name: service-overview
      namespace: my-service
    spec:
      instanceSelector:
        matchLabels:
          micromegas.info/env: prod
      name: service-overview        # optional, defaults to metadata.name; immutable
      screenType: notebook          # optional, default; immutable
      folderPath: my-service/prod   # optional; the folder is created implicitly
      config:                       # exactly one of config / configFrom
        timeRangeFrom: now-1h
        timeRangeTo: now
        cells: []
      # configFrom:
      #   configMapKeyRef:
      #     name: my-service-screens
      #     key: overview.json

`config` has the same shape as the `config` field in the REST API and in the
`micromegas-screens` export files. `configFrom` reads one ConfigMap key holding that JSON, which
suits Helm charts that keep dashboards as files (`.Files.Glob "screens/*.json"`).

The screen name must follow the server's rules: 3 to 100 characters, lowercase letters, digits and
single hyphens, starting with a letter. Use `spec.name` when the CR name does not qualify.

### Ownership

The operator only writes screens whose `managed_by` is its own marker, or that do not exist yet.
If a screen with the same name exists and was created by hand or by `micromegas-screens`, the CR
reports `Ready=False` with reason `Conflict` and nothing is changed. Resolve it by deleting or
renaming the server screen, or by setting its `managed_by` to the marker the CR expects.

Edits made in the web app to a managed screen are reverted on the next resync.

Deleting the `Screen` CR deletes the server screen. A finalizer holds the CR until the delete
succeeds; if the instance is unreachable the CR stays in `Terminating` until it is.

### Status

`status.conditions` carries one `Ready` condition. Reasons:

| Reason | Meaning |
|---|---|
| `Synced` | Every selected instance has the desired screen |
| `NoMatchingInstance` | No `MicromegasInstance` matches `instanceSelector` |
| `InstanceNotReady` | The instance is not `Ready`; fix the instance first |
| `InvalidName` | `spec.name` or `folderPath` fails the server's rules |
| `InvalidConfig` | The server rejected the config, or the ConfigMap value is not a JSON object |
| `ConfigMapNotFound` | `configFrom` names a missing ConfigMap or key |
| `Conflict` | A screen with that name exists and is not owned by this CR |
| `ApiError` | Network or server error; retried with backoff |

`status.instances` lists each instance with the screen name, a hash of the config sent, and the
last sync time.

## Operator configuration

| Helm value | Env var | Meaning |
|---|---|---|
| `clusterName` | `MICROMEGAS_OPERATOR_CLUSTER_NAME` | Required; part of `managed_by` |
| `watchNamespace` | `MICROMEGAS_OPERATOR_WATCH_NAMESPACE` | Restrict to one namespace; empty means all |
| `healthPort` | `MICROMEGAS_OPERATOR_HEALTH_LISTEN` | `/healthz` and `/readyz` |
| `telemetry.url` | `MICROMEGAS_TELEMETRY_URL` | Send the operator's own logs and metrics to an ingestion server |
| `telemetry.apiKeySecret` | `MICROMEGAS_INGESTION_API_KEY` | Ingestion key for the above |

The operator runs one replica with a `Recreate` strategy.

## Local development

`local_test_env/ai_scripts/operator_e2e.py` starts the monolith with auth disabled, creates a kind
cluster, runs the operator from source, and exercises create, update, resync, conflict, and delete.
```

- [ ] **Step 2: Nav and cross-link**

In `mkdocs/mkdocs.yml`, after the line `- Analytics Web App Deployment: admin/web-app.md` inside the Admin section (around line 153), add:
```yaml
        - Kubernetes Operator: admin/kubernetes-operator.md
```

In `mkdocs/docs/web-app/notebooks/screens-as-code.md`, at the end of the `## Overview` section, add the paragraph:

```markdown
Teams deploying on Kubernetes can instead declare screens as `Screen` custom resources and let
the [Kubernetes operator](../../admin/kubernetes-operator.md) keep the server in sync.
```

- [ ] **Step 3: CHANGELOG**

Add to `CHANGELOG.md` under `## Unreleased`, as the first bullet:

```markdown
* **Kubernetes:** New `micromegas-operator` (`rust/micromegas-operator`, image
  `marcantoinedesroches/micromegas-operator`, chart `charts/micromegas-operator`) reconciling
  `Screen` and `MicromegasInstance` custom resources (`micromegas.info/v1alpha1`) into
  analytics-web-srv screens over the existing REST API. Ownership reuses `screens.managed_by`,
  stamped `k8s://<cluster>/<namespace>/<name>`; screens owned by anyone else are reported as
  `Conflict` and never overwritten. Auth is OIDC client credentials from a Secret. See
  `mkdocs/docs/admin/kubernetes-operator.md`. **Minor breaking change:** the screen wire types
  (`Screen`, `CreateScreenRequest`, `UpdateScreenRequest`, `ErrorResponse`), `ScreenType`, and the
  name/folder validators moved from `analytics-web-srv` into the new `analytics-web-api` crate;
  `analytics_web_srv::app_db` and `analytics_web_srv::screen_types` re-export them, and
  `ErrorResponse`'s fields are now public.
```

- [ ] **Step 4: Docs site check**

Run: `python3 build/check_docs_site.py` (if it requires a built site, follow its `--help`). Also `cd mkdocs && mkdocs build --strict` if mkdocs is installed in the poetry venv (`cd python/micromegas && poetry run mkdocs build --strict -f ../../mkdocs/mkdocs.yml`).
Expected: no broken links, new page in nav.

- [ ] **Step 5: Final workspace CI and commit**

Run: `python3 build/rust_ci.py native`
Expected: all steps pass including "CRD Freshness Check".

```bash
git add mkdocs CHANGELOG.md
git commit -m "Document the Kubernetes operator"
git log --oneline main..HEAD
```

---

## Self-Review Notes

- **Spec coverage.** Shared crate → Task 1. CRDs, crdgen, CI freshness → Task 2. Ownership/plan semantics → Task 4. Auth → Task 5. Client → Task 6. Instance controller, Secret watch, token cache invalidation → Tasks 7, 9. Screen controller, finalizer, ConfigMap source, events, resync, backoff → Tasks 8, 9. Health, telemetry → Task 9. Helm chart, RBAC, Recreate, examples → Task 10. Image → Task 11. E2E → Task 12. Docs, changelog → Task 13. Follow-ups stay follow-ups.
- **Deviation recorded** in Global Constraints: single `--watch-namespace` instead of a list.
- **Type consistency.** `Failure`, `Identity`, `InstanceOutcome`, `Desired`, `Action`, `Credentials`, `ApiError`, `TokenError`, `BuildClientError`, `Error`, `Backoff` are named identically across Tasks 4–9. `reasons::*` constants are the only reason strings used.
