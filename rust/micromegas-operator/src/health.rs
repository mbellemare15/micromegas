//! Liveness/readiness HTTP endpoints for the operator's Kubernetes probes.

use axum::http::StatusCode;
use axum::{Router, routing::get};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `ready` is set once the instance reflector has completed its initial LIST;
/// until then the screen controller is held back, so the pod is not ready.
pub fn router(ready: Arc<AtomicBool>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/readyz",
            get(move || {
                let ready = ready.clone();
                async move {
                    if ready.load(Ordering::SeqCst) {
                        (StatusCode::OK, "ok")
                    } else {
                        (StatusCode::SERVICE_UNAVAILABLE, "syncing")
                    }
                }
            }),
        )
}

pub async fn serve(addr: SocketAddr, ready: Arc<AtomicBool>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(ready)).await?;
    Ok(())
}
