# micromegas-operator

Installs the CRDs, RBAC, and Deployment for the Micromegas Kubernetes operator.

    helm install micromegas-operator ./charts/micromegas-operator \
      --namespace micromegas-system --create-namespace \
      --set clusterName=prod-eu

Helm installs `crds/` on first install only. On upgrade, apply them by hand:

    kubectl apply -f charts/micromegas-operator/crds/

See `examples/` for a `MicromegasInstance` and two `Screen` shapes, and
https://micromegas.info/docs/admin/kubernetes-operator/ for the full reference.
