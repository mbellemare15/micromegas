//! Tests for micromegas_operator::health.

use micromegas_operator::health;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn readyz_is_unavailable_until_the_stores_sync() {
    let ready = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind health listener");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("health listener address")
    );
    let app = health::router(ready.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let http = reqwest::Client::new();
    let get = |path: String| {
        let http = http.clone();
        async move {
            http.get(path)
                .send()
                .await
                .expect("health request")
                .status()
        }
    };

    assert_eq!(get(format!("{base}/healthz")).await, 200);
    assert_eq!(get(format!("{base}/readyz")).await, 503);
    ready.store(true, Ordering::SeqCst);
    assert_eq!(get(format!("{base}/readyz")).await, 200);
}
