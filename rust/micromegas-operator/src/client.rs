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
