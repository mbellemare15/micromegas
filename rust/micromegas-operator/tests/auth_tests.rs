//! Tests for micromegas_operator::auth.

use micromegas_operator::auth::{OidcClientCredentials, TokenCache, TokenError};
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
    assert!(matches!(
        cache.bearer().await,
        Err(TokenError::Discovery(_))
    ));
}

#[test]
fn fingerprint_changes_with_any_field() {
    let a = creds("https://i".into(), None);
    let mut b = creds("https://i".into(), None);
    assert_eq!(a.fingerprint(), b.fingerprint());
    b.client_secret = "other".into();
    assert_ne!(a.fingerprint(), b.fingerprint());
}
