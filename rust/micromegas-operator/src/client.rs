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
        match self
            .send(Method::GET, &format!("screens/{name}"), None::<&()>)
            .await
        {
            Ok(screen) => Ok(Some(screen)),
            Err(ApiError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn create_screen(&self, req: &CreateScreenRequest) -> Result<Screen, ApiError> {
        self.send(Method::POST, "screens", Some(req)).await
    }

    pub async fn update_screen(
        &self,
        name: &str,
        req: &UpdateScreenRequest,
    ) -> Result<Screen, ApiError> {
        self.send(Method::PUT, &format!("screens/{name}"), Some(req))
            .await
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
            serde_json::from_str(body).unwrap_or_else(|_| ErrorResponse::new("HTTP_400", body)),
        ),
        _ => ApiError::Transient(format!("HTTP {status}: {body}")),
    }
}

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
        let c = WebApiClient::new(
            reqwest::Client::new(),
            "https://h/telemetry/",
            Credentials::None,
        );
        assert_eq!(c.api_url("screens"), "https://h/telemetry/api/screens");
        let root = WebApiClient::new(reqwest::Client::new(), "https://h", Credentials::None);
        assert_eq!(root.api_url("screens/x"), "https://h/api/screens/x");
    }

    #[tokio::test]
    async fn get_screen_found_and_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/screens/overview"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(screen_json(Some("k8s://c/ns/n"))),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/screens/nope"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"code": "NOT_FOUND", "message": "x"})),
            )
            .mount(&server)
            .await;
        let c = client(&server);
        let found = c.get_screen("overview").await.unwrap().unwrap();
        assert_eq!(found.managed_by.as_deref(), Some("k8s://c/ns/n"));
        assert!(c.get_screen("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_posts_body_and_maps_duplicate_to_bad_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/screens"))
            .and(body_partial_json(
                json!({"name": "overview", "managed_by": "k8s://c/ns/n", "folder_path": "team"}),
            ))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(screen_json(Some("k8s://c/ns/n"))),
            )
            .mount(&server)
            .await;
        let c = client(&server);
        let req = CreateScreenRequest {
            name: "overview".into(),
            screen_type: "notebook".into(),
            config: json!({"cells": []}),
            managed_by: Some("k8s://c/ns/n".into()),
            folder_path: "team".into(),
        };
        assert_eq!(c.create_screen(&req).await.unwrap().name, "overview");

        let server2 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/screens"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"code": "DUPLICATE_NAME", "message": "exists"})),
            )
            .mount(&server2)
            .await;
        match client(&server2).create_screen(&req).await {
            Err(ApiError::BadRequest(e)) => assert_eq!(e.code, "DUPLICATE_NAME"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_puts_partial_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/screens/overview"))
            .and(body_partial_json(json!({"folder_path": "new"})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(screen_json(Some("k8s://c/ns/n"))),
            )
            .mount(&server)
            .await;
        let req = UpdateScreenRequest {
            config: None,
            managed_by: None,
            folder_path: Some("new".into()),
        };
        client(&server)
            .update_screen("overview", &req)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_treats_404_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/screens/overview"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        client(&server).delete_screen("overview").await.unwrap();
    }

    #[tokio::test]
    async fn status_mapping() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/screen-types"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server).probe().await,
            Err(ApiError::Unauthorized)
        ));

        let server2 = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/screen-types"))
            .respond_with(ResponseTemplate::new(503).set_body_string("down"))
            .mount(&server2)
            .await;
        let err = client(&server2).probe().await.unwrap_err();
        assert!(err.is_transient(), "{err:?}");

        let unreachable = WebApiClient::new(
            reqwest::Client::new(),
            "http://127.0.0.1:9",
            Credentials::None,
        );
        assert!(unreachable.probe().await.unwrap_err().is_transient());
    }

    #[tokio::test]
    async fn sends_bearer_from_token_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/realm/.well-known/openid-configuration"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"token_endpoint": format!("{}/realm/token", server.uri())}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/realm/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "tok", "expires_in": 600})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/screen-types"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;
        let cache = Arc::new(TokenCache::new(
            reqwest::Client::new(),
            OidcClientCredentials {
                issuer: format!("{}/realm", server.uri()),
                client_id: "a".into(),
                client_secret: "b".into(),
                audience: None,
            },
        ));
        let c = WebApiClient::new(
            reqwest::Client::new(),
            &server.uri(),
            Credentials::Oidc(cache),
        );
        c.probe().await.unwrap();
    }
}
