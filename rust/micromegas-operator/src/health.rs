//! Liveness/readiness HTTP endpoints for the operator's Kubernetes probes.

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
