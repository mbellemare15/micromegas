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
import shutil
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


def check_prerequisites():
    missing = [tool for tool in ("docker", "kind", "kubectl", "cargo") if shutil.which(tool) is None]
    if missing:
        raise SystemExit(f"Missing prerequisites on PATH: {', '.join(missing)}")


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
    check_prerequisites()
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
