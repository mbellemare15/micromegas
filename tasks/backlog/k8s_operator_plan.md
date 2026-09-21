# Kubernetes Operator for Screens — Design

## Overview

A Kubernetes operator, `micromegas-operator`, that reconciles `Screen` custom resources into
screens on a Micromegas analytics web server, the way grafana-operator reconciles
`GrafanaDashboard` into a Grafana. A second custom resource, `MicromegasInstance`, names a target
server and its credentials; `Screen` picks its instance(s) with a label selector.

The goal is GitOps for dashboards: a service's Helm chart ships its notebook screens next to its
Deployment, ArgoCD applies them, and the operator keeps the server in sync. Screens edited by hand
in the UI drift back on the next resync; screens created by hand and not owned by the operator are
never touched.

This is a client of the existing REST API in `rust/analytics-web-srv`. The only server-side change
is a refactor: the request/response types and validators that both the server and the operator need
move into a small shared crate so the CRD schema cannot drift from what the server accepts.

## Current State

**Screens are the dashboards.** The saved unit is a screen (`rust/analytics-web-srv/src/app_db/models.rs:10`):
`name` (primary key, slug), `screen_type`, `config` (free-form JSON), `folder_path`,
`managed_by: Option<String>`, plus `created_by`/`updated_by`/timestamps. `notebook` is the only
creatable type (`screen_types.rs`); the four other types are deprecated. `name` is immutable and is
also the URL. It is validated by `validate_name` (`models.rs:276`): 3–100 chars, `[a-z0-9-]`,
starts with a letter, no consecutive hyphens, `new` reserved.

**Folders are path prefixes, not objects** (`rust/analytics-web-srv/src/folders.rs:1-4`). A folder
exists if it has a row in `folders` or is a prefix of some screen's `folder_path`. Setting
`folder_path` on a screen is enough to make its folder appear in the UI, and an implicit folder
disappears when its last screen leaves. Folders carry no `managed_by` or other attributes. This is
why there is no `Folder` resource in v1 (see Follow-ups).

**`managed_by` is an existing ownership marker.** The `micromegas-screens` CLI stamps it with the git
remote URL on every create/update and only deletes server screens whose `managed_by` equals its
own (`python/micromegas/micromegas/cli/screens.py`). The web UI shows a "managed by source control"
banner when it is set. `PUT /api/screens/{name}` uses COALESCE semantics, so `managed_by` can be
changed but not cleared.

**REST API** under `{MICROMEGAS_BASE_PATH}/api` (`web_server.rs:396-445`):
`GET/POST /screens`, `GET/PUT/DELETE /screens/{name}`, `GET /screen-types`,
`GET/POST/PUT/DELETE /folders`. Errors are `{code, message}`. There is no FlightSQL path for
screens.

**Auth.** The web server accepts only OIDC bearer JWTs (`auth/handlers.rs`); there is no API-key path
in this crate. The Python `OidcClientCredentialsProvider` (`auth/oidc.py:584`) performs the
client-credentials grant and sends the returned `access_token` as the bearer; the server validates
it against the issuers listed in `MICROMEGAS_OIDC_CONFIG`. Screen handlers require only
an authenticated principal — there is no per-screen authorization and no admin check.

**Nothing exists on the Kubernetes side.** No Helm chart, no manifests. Dockerfiles live in
`docker/*.Dockerfile`. The only Go in the repo is the Grafana datasource plugin.

## Decisions

| Question | Decision | Why |
|---|---|---|
| Language | Rust, kube-rs | Matches the repo; shares types and validators with the server |
| Instance targeting | `MicromegasInstance` CR + `instanceSelector` | grafana-operator model; several servers per cluster; per-space separation |
| Auth (v1) | OIDC client credentials from a Secret | Works today with no server change; mirrors the Python machine client |
| Scope (v1) | `Screen` only | One API, one credential type; folders are implicit path prefixes; view sets and data sources are follow-ups |
| Config carrier | Inline object **or** ConfigMap key reference | Inline is Helm-templatable and diffable; ConfigMap suits `.Files.Get` workflows |
| Reconcile model | Per-resource controllers, `managed_by` ownership, finalizers | Fine-grained status; conflicts reported, not overwritten; orphan sweep is a follow-up |

## Repository Layout

```
rust/analytics-web-api/           # NEW lib crate: shared DTOs + validators (see §Shared crate)
rust/analytics-web-srv/           # depends on analytics-web-api instead of local definitions
rust/micromegas-operator/         # NEW bin crate
  src/main.rs                     # clap flags, tracing init, spawns the two controllers
  src/bin/crdgen.rs               # prints CRD YAML for charts/micromegas-operator/crds/
  src/crds/{instance,screen}.rs   # CustomResource structs + status types
  src/client.rs                   # WebApiClient over reqwest, typed by analytics-web-api
  src/auth.rs                     # OIDC client-credentials token cache
  src/reconcile/{instance,screen}.rs
  src/plan.rs                     # pure desired-vs-actual diff (unit tested)
  src/conditions.rs               # Ready condition helpers
charts/micromegas-operator/       # NEW Helm chart (CRDs, Deployment, RBAC, SA)
docker/micromegas-operator.Dockerfile
mkdocs/docs/admin/kubernetes-operator.md
```

The workspace globs `rust/*`, so both new crates join automatically.

## Shared crate: `analytics-web-api`

Extracted from `analytics-web-srv`, no behavior change:

- `ScreenType` enum and its serde names.
- `CreateScreenRequest`, `UpdateScreenRequest`, `ScreenResponse` (the wire shape of a screen),
  `ErrorResponse { code, message }`.
- `validate_name`, `normalize_name`, `validate_folder_path`, `ValidationError`
  (`validate_folder_path` is needed for `spec.folderPath`).

`analytics-web-srv` re-exports or imports these; handlers and `app_db::models` keep their DB row
types locally. The operator depends on this crate and never on `analytics-web-srv`. Record the
move in `CHANGELOG.md` under the Rust-API "Minor breaking change" clause.

## Custom Resources

API group `micromegas.info`, version `v1alpha1`. Both kinds are namespaced. CRDs are generated
with `kube::CustomResourceExt` from the Rust structs (schemars), committed under
`charts/micromegas-operator/crds/`, and CI fails if `crdgen` output differs from the committed files.

### MicromegasInstance

```yaml
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata:
  name: prod
  namespace: micromegas
  labels: { micromegas.info/env: prod }
spec:
  url: https://micromegas.example.com/telemetry   # web app base URL, including MICROMEGAS_BASE_PATH
  auth:                                            # optional; omitted = no Authorization header
    oidcClientCredentials:
      issuer: https://idp.example.com/realms/example
      audience: micromegas                         # optional
      secretRef:
        name: micromegas-operator-oidc
        clientIdKey: client_id                     # default
        clientSecretKey: client_secret             # default
  resyncInterval: 10m                              # default 10m; minimum 1m
status:
  observedGeneration: 3
  conditions:
    - type: Ready
      status: "True"
      reason: Connected
      lastTransitionTime: ...
```

- Omitting `auth` sends no token. This only works against a server started with `--disable-auth`
  and exists for local development and the end-to-end script. The docs say so.
- The instance reconciler resolves the Secret, obtains a token, calls `GET /api/screen-types`, and
  sets `Ready`. Reasons on failure: `SecretNotFound`, `TokenError`, `Unreachable`, `Unauthorized`.
- The reconciler watches the referenced Secret; a Secret change re-enqueues the instance and drops
  the cached token.

### Screen

```yaml
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata:
  name: velocity-overview
  namespace: game-system
spec:
  instanceSelector:
    matchLabels: { micromegas.info/env: prod }
  name: velocity-overview          # optional; defaults to metadata.name
  screenType: notebook             # optional; default notebook; immutable (CEL rule)
  folderPath: velocity/prod        # optional; default "" (root)
  config:                          # exactly one of config / configFrom (CEL rule)
    timeRangeFrom: now-1h
    timeRangeTo: now
    cells: [ ... ]
  # configFrom:
  #   configMapKeyRef: { name: velocity-screens, key: overview.json }
status:
  observedGeneration: 5
  conditions:
    - type: Ready
      status: "True"
      reason: Synced
  instances:
    - name: prod
      namespace: micromegas
      screenName: velocity-overview
      configHash: sha256:...
      lastSyncedAt: ...
```

- `spec.config` is `x-kubernetes-preserve-unknown-fields`; the operator does not validate notebook
  internals, the server does. A server `400` becomes `Ready=False / InvalidConfig` with the server's
  message.
- `spec.name` is validated by the shared `validate_name` at reconcile time (`InvalidName`). A CEL
  pattern on the CRD catches the cheap cases at admission; the reconciler is authoritative.
- `Ready` reasons: `Synced`, `Conflict`, `NoMatchingInstance`, `InstanceNotReady`, `InvalidName`,
  `InvalidConfig`, `ConfigMapNotFound`, `ApiError`.
- When the selector matches several instances, `Ready=True` only if every instance synced;
  `status.instances` reports each one.
- `spec.folderPath` is validated with the shared `validate_folder_path` (`InvalidName`). The
  operator never calls the folders endpoints: the server materializes the folder from the path,
  and an implicit folder vanishes when its last screen is deleted.

## Ownership and Reconcile Semantics

**Identity.** Every screen the operator writes has
`managed_by = "k8s://<cluster-name>/<namespace>/<cr-name>"`. `--cluster-name` is a required
operator flag (Helm value `clusterName`). The `k8s://` prefix distinguishes operator ownership from
the screens CLI's git URLs and gives a future orphan sweeper a stable prefix to filter on.

**Screen reconcile**, per matched instance:

1. Resolve config: inline `spec.config`, or read the ConfigMap key and parse it as JSON
   (`ConfigMapNotFound` / `InvalidConfig` on failure).
2. `GET /api/screens/{name}`.
3. Apply the pure `plan(desired, current) -> Action`:
   - current is `404` → `Create` (POST with `managed_by` set).
   - current `managed_by == ours` → `Update` if normalized `config` or `folder_path` differ, else `NoOp`.
     `screen_type` differing from spec → `Conflict` (server-side immutable; only `notebook` is creatable,
     so recreate is not offered in v1).
   - current `managed_by` is empty or someone else's → `Conflict`. Nothing is written.
4. Write status and emit a Kubernetes Event for Create, Update, Delete, and Conflict.

"Normalized" means comparing `serde_json::Value` after parsing, so key order and whitespace never
cause writes. `configHash` in status is the hash of the normalized config actually sent.

**Deletion.** A finalizer `micromegas.info/screen` deletes the server screen on every instance whose
`managed_by` is still ours, then removes itself. If the instance is gone or unreachable, the
operator retries with backoff; `kubectl delete --force` semantics apply if a user strips the
finalizer, which is what the follow-up orphan sweeper exists to clean up.

**Drift.** Every `resyncInterval` the Screen is re-enqueued, so UI edits to a managed screen are
reverted. The resync interval comes from the instance, not the screen.

**Cross-namespace selection.** A `Screen` in any watched namespace may select an instance in any
other namespace. The server has no per-screen authorization to enforce anyway, so the v1 boundary
is "who can create `Screen` objects in the cluster", governed by Kubernetes RBAC. Restricting
selection to same-namespace or an allow-list is a documented follow-up.

**Conflicts are sticky by design.** A `Conflict` is not retried on a timer; it clears on the next
spec change or resync once the server-side screen has been deleted, renamed, or its `managed_by`
changed to ours (for example by a one-time `micromegas-screens`-style import step).

## Controller Mechanics

- Two `kube::runtime::Controller`s in one process:
  - Instance controller: owns `MicromegasInstance`, watches `Secret` (mapped through
    `spec.auth.*.secretRef`).
  - Screen controller: owns `Screen`, watches `MicromegasInstance` (any change re-enqueues every
    Screen whose selector matches) and `ConfigMap` (mapped to Screens whose `configFrom` names it).
- `kube::runtime::finalizer` on `Screen`.
- Status via server-side apply on the `status` subresource with a fixed field manager.
- Error policy: transient errors (network, 5xx, 429) requeue with the runtime's exponential backoff
  and set `Ready=False / ApiError` with the message; 4xx validation errors do not requeue until spec
  change or resync.
- Token cache per instance keyed by instance UID; refresh 60 s before expiry; single in-flight
  refresh per instance.
- One replica, `strategy: Recreate`. No leader election in v1.
- `/healthz` and `/readyz` on a small axum listener for probes.
- The operator uses `micromegas-tracing` for its own logs, metrics, and spans, configured through the
  same environment variables as the other services, so it can report into the Micromegas it manages.

## Helm Chart

`charts/micromegas-operator/`:

- `crds/` with the generated CRDs (installed by Helm's CRD mechanism; documented upgrade note that
  Helm does not upgrade CRDs, with a `kubectl apply -f crds/` step).
- `Deployment`, `ServiceAccount`, `ClusterRole` + `ClusterRoleBinding` (or `Role`s when
  `watchNamespaces` is set): `get/list/watch/update/patch` on the two CRDs, their `status` and
  `finalizers`; `get/list/watch` on `configmaps` and `secrets`; `create/patch` on `events`.
- Values: `image`, `clusterName` (required), `watchNamespaces` (empty = all), `defaultResyncInterval`,
  `resources`, `telemetry` (ingestion URL and key for self-reporting, optional).

Consumer pattern for the ArgoCD layout: a service umbrella chart adds `templates/screens.yaml` with
one `Screen` per dashboard, either inline `config` or a `ConfigMap` built from
`.Files.Glob "screens/*.json"` plus `configFrom` references.

## Error Handling Summary

| Situation | Behavior | Status |
|---|---|---|
| No instance matches selector | no action | `Ready=False / NoMatchingInstance` |
| Instance not Ready | skip, re-enqueue on instance change | `Ready=False / InstanceNotReady` |
| Secret missing / token error | instance not Ready | instance `SecretNotFound` / `TokenError` |
| Server 401/403 | instance not Ready, screens `InstanceNotReady` | `Unauthorized` |
| Screen name or folder path invalid | no action | `InvalidName` |
| Server 400 on create/update | no retry until spec change | `InvalidConfig` (server message) |
| Existing screen not ours | no action | `Conflict` |
| ConfigMap or key missing | no action, re-enqueue on ConfigMap change | `ConfigMapNotFound` |
| Network / 5xx | exponential backoff | `ApiError` |

## Testing

- **Unit** (`rust/micromegas-operator`): `plan()` for every branch (create, update on config
  change, update on folder change, no-op on reordered keys, conflict on empty/foreign `managed_by`,
  conflict on screen_type mismatch); `managed_by` formatting; config normalization; condition
  transitions; selector matching.
- **Shared crate**: existing validator tests move with the code.
- **Client**: `wiremock` tests for `WebApiClient` — 404 → `None`, 400 → typed `ErrorResponse`,
  401 → `Unauthorized`, 5xx → transient, token refresh on expiry.
- **CRD snapshot**: `crdgen` output committed; CI diff check.
- **End-to-end (manual, scripted)**: `local_test_env/ai_scripts/operator_e2e.py` starts the monolith
  with `--disable-auth`, creates a `kind` cluster, installs the chart, applies a sample
  `MicromegasInstance` (no `auth`) and two `Screen`s (inline and ConfigMap) in a nested folder path,
  then checks the server via `GET /api/screens` and `GET /api/folders` (the folder must appear
  implicitly); edits a screen through the API and
  verifies it reverts after resync; deletes a `Screen` and verifies removal; pre-creates an unmanaged
  screen and verifies `Conflict`. Not checked in as a `#[ignore]` test, per `CONTRIBUTING.md`.

## Documentation

- `mkdocs/docs/admin/kubernetes-operator.md`: install, `MicromegasInstance` setup including the
  OIDC service account and the `MICROMEGAS_OIDC_CONFIG` entry, CRD reference, ownership and conflict
  semantics, Helm consumer example, troubleshooting by `Ready` reason.
- Cross-link from `mkdocs/docs/web-app/notebooks/screens-as-code.md` ("or use the operator").
- `CHANGELOG.md`: new operator; shared crate extraction as a minor Rust API break.

## Follow-ups (explicitly out of v1)

1. **Orphan sweeper**: per-instance periodic job deleting screens whose `managed_by` starts with
   `k8s://<cluster-name>/` and has no matching `Screen` CR.
2. **`ViewSet` CR**: materialized view DDL over FlightSQL (what `micromegas-views` does), requiring
   a FlightSQL client and a `lakehouse_admin` credential on the instance.
3. **`DataSource` CR** for the `data_sources` table, if a REST surface is added for it.
4. **Kubernetes service-account token auth**: projected token with a custom audience; the cluster
   issuer listed in `MICROMEGAS_OIDC_CONFIG`.
5. **`deletionPolicy: Retain`** on `Screen`: needs an API change so `PUT` can clear `managed_by`.
6. **`adopt: true`** on `Screen` to take over an unmanaged screen instead of reporting `Conflict`.
7. **Leader election** (`kube-lease-manager`) for multi-replica deployments.
8. **Namespace restriction** on cross-namespace instance selection.
9. **Server-side audit records** for screen mutations in `mutation_audit.rs`, which today only
   covers grants and groups.
10. **`Folder` CR**, only if folders gain server-side state worth managing (description, default
    time range, permissions, an owner marker). Today a folder is a path prefix materialized by its
    screens, so a `Folder` resource would only pre-create empty folders and could not tell its own
    folders from anyone else's. Adding the CRD later is additive; removing one is not.
