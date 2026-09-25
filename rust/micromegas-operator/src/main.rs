//! Kubernetes operator: reconciles Screen custom resources into analytics-web-srv screens.
//!
//! Telemetry is configured by `micromegas_main` from the usual env vars
//! (`MICROMEGAS_TELEMETRY_URL`, `MICROMEGAS_INGESTION_API_KEY`, ...), so the
//! operator can report into the Micromegas it manages.

use clap::Parser;
use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::runtime::controller::Controller;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::reflector::ObjectRef;
use kube::runtime::watcher;
use kube::{Api, Client, ResourceExt};
use micromegas::micromegas_main;
use micromegas::tracing::prelude::*;
use micromegas_operator::crds::{MicromegasInstance, Screen};
use micromegas_operator::health;
use micromegas_operator::reconcile::instance::matching_instances;
use micromegas_operator::reconcile::{Backoff, Context, FIELD_MANAGER, instance, screen};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser, Debug)]
#[clap(
    name = "micromegas-operator",
    about = "Kubernetes operator for Micromegas screens",
    version
)]
struct Cli {
    /// Stable name of this cluster; part of every managed screen's managed_by marker.
    #[clap(long, env = "MICROMEGAS_OPERATOR_CLUSTER_NAME")]
    cluster_name: String,

    /// Restrict watches to one namespace. Default: all namespaces.
    #[clap(long, env = "MICROMEGAS_OPERATOR_WATCH_NAMESPACE")]
    watch_namespace: Option<String>,

    #[clap(
        long,
        env = "MICROMEGAS_OPERATOR_HEALTH_LISTEN",
        default_value = "0.0.0.0:8080"
    )]
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
    let configmaps_api: Api<ConfigMap> = api(&client, ns);

    let instance_ctrl = Controller::new(instances_api.clone(), watcher::Config::default());
    let instances_store = instance_ctrl.store();
    let screen_ctrl = Controller::new(screens_api, watcher::Config::default());
    let screens_store = screen_ctrl.store();

    let instances_ready = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(Context {
        client: client.clone(),
        http: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?,
        cluster_name: cli.cluster_name.clone(),
        instances: instances_store.clone(),
        tokens: Mutex::new(Default::default()),
        recorder: Recorder::new(
            client.clone(),
            Reporter {
                controller: FIELD_MANAGER.into(),
                instance: std::env::var("HOSTNAME").ok(),
            },
        ),
        backoff: Backoff::default(),
        instances_ready: instances_ready.clone(),
    });

    // Instance changes re-enqueue every Screen whose selector matches it.
    let screens_for_instances = screens_store.clone();
    let screens_for_configmaps = screens_store.clone();
    let screen_ctrl = screen_ctrl
        .watches(
            instances_api,
            watcher::Config::default(),
            move |inst: MicromegasInstance| {
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
            },
        )
        .watches(
            configmaps_api,
            watcher::Config::default(),
            move |cm: ConfigMap| {
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
            },
        )
        .shutdown_on_signal();

    // No Secret watch: watching Secrets cluster-wide would cache every Secret
    // body in the operator. The instance probe re-reads the referenced Secret
    // every PROBE_INTERVAL and replaces the token cache when the credentials
    // change, so a rotation takes effect within that interval.
    let instance_ctrl = instance_ctrl.shutdown_on_signal();

    info!(
        "micromegas-operator starting, cluster_name={}",
        cli.cluster_name
    );

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

    // The screen controller's cleanup path reads the instance store to find the
    // servers holding a screen. Running it before the initial LIST lands would
    // let a delete processed at startup drop the finalizer while the server
    // screen is still there, so the screen controller and /readyz both wait for
    // the instance reflector.
    let gated_screen_loop = async {
        if instances_store.wait_until_ready().await.is_err() {
            return;
        }
        instances_ready.store(true, Ordering::SeqCst);
        info!("instance store synced, starting screen controller");
        screen_loop.await;
    };

    tokio::select! {
        _ = instance_loop => {},
        _ = gated_screen_loop => {},
        r = health::serve(cli.health_listen, ctx.instances_ready.clone()) => { r?; },
    }
    Ok(())
}
