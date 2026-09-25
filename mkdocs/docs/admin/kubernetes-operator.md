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

An empty `instanceSelector` matches no instance, unlike the Kubernetes convention where it matches
everything, so a Screen always opts in to the instances it writes to. Selection is cross-namespace:
a Screen in one namespace can select an instance in another, which makes the security boundary who
may create `Screen` objects rather than namespace isolation.

### Ownership

The operator only writes screens whose `managed_by` is its own marker, or that do not exist yet.
If a screen with the same name exists and was created by hand or by `micromegas-screens`, the CR
reports `Ready=False` with reason `Conflict` and nothing is changed. Resolve it by deleting or
renaming the server screen, or by setting its `managed_by` to the marker the CR expects.

Edits made in the web app to a managed screen are reverted on the next resync.

Deleting a `MicromegasInstance` while Screens still select it leaves the server screens in place;
those CRs flip to `Ready=False` with reason `NoMatchingInstance`.

Deleting the `Screen` CR deletes the server screen. A finalizer holds the CR until the delete
succeeds; if the instance is unreachable the CR stays in `Terminating` until it is.

Forcing deletion: if the instance stays unreachable and you need the CR gone anyway, remove the
finalizer directly. This leaves the server screen in place.

    kubectl patch screen <name> -p '{"metadata":{"finalizers":null}}' --type=merge

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

A 401/403 from the server on a screen write is reported as `ApiError`, with a message naming the
unauthorized (401/403) response and pointing at the instance's credentials.

`status.instances` lists each instance the screen was synced to, with `name`, `namespace`,
`screenName`, a `configHash` of the config sent, `lastSyncedAt`, and an `error` field when that
instance failed.

## Shipping screens from a service chart

A service keeps its dashboards as JSON files under `screens/` in its own chart and renders one
ConfigMap plus one `Screen` per file:

    # templates/screens.yaml
    apiVersion: v1
    kind: ConfigMap
    metadata:
      name: {{ .Release.Name }}-screens
    data:
      {{- range $path, $_ := .Files.Glob "screens/*.json" }}
      {{ base $path }}: |
        {{- $.Files.Get $path | nindent 4 }}
      {{- end }}
    {{- range $path, $_ := .Files.Glob "screens/*.json" }}
    ---
    apiVersion: micromegas.info/v1alpha1
    kind: Screen
    metadata:
      name: {{ printf "%s-%s" $.Release.Name (base $path | trimSuffix ".json") }}
    spec:
      instanceSelector:
        matchLabels: {{- toYaml $.Values.micromegas.instanceSelector | nindent 10 }}
      folderPath: {{ $.Values.micromegas.folderPath | quote }}
      configFrom:
        configMapKeyRef:
          name: {{ $.Release.Name }}-screens
          key: {{ base $path }}
    {{- end }}

with values such as:

    micromegas:
      instanceSelector:
        micromegas.info/env: prod
      folderPath: my-service/prod

Each file's name becomes the ConfigMap key and part of the `Screen` name, so adding a dashboard is
one new file. The operator deletes the server screen when the `Screen` object goes away, which makes
`helm uninstall` remove the dashboards too.

## Operator configuration

| Helm value | Env var | Meaning |
|---|---|---|
| `clusterName` | `MICROMEGAS_OPERATOR_CLUSTER_NAME` | Required; part of `managed_by` |
| `watchNamespace` | `MICROMEGAS_OPERATOR_WATCH_NAMESPACE` | Restrict to one namespace; empty means all |
| `healthPort` | `MICROMEGAS_OPERATOR_HEALTH_LISTEN` | Port for `/healthz` and `/readyz`; the chart renders it as `0.0.0.0:<healthPort>` into `MICROMEGAS_OPERATOR_HEALTH_LISTEN` |
| `telemetry.url` | `MICROMEGAS_TELEMETRY_URL` | Send the operator's own logs and metrics to an ingestion server |
| `telemetry.apiKeySecret` | `MICROMEGAS_INGESTION_API_KEY` | Ingestion key for the above |

The operator runs one replica with a `Recreate` strategy.

With `watchNamespace` empty the operator lists and watches `Screen`, `MicromegasInstance`, and
`ConfigMap` objects cluster-wide, and reads the Secrets referenced by the instances it manages.
Secrets are read on demand, never watched: a rotated credential is picked up by the next instance
probe, so within five minutes.

## Local development

`local_test_env/ai_scripts/operator_e2e.py` starts the monolith with auth disabled, creates a kind
cluster, builds and loads the operator image, installs the chart into `micromegas-system`, checks
the service account's permissions, and exercises create, update, resync, conflict, and delete.
