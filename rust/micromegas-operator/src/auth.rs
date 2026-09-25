//! OIDC client-credentials grant, cached per instance. The access token is sent
//! as the bearer, which is what analytics-web-srv validates (same as the Python
//! machine client).

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const DEFAULT_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcClientCredentials {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub audience: Option<String>,
}

impl OidcClientCredentials {
    /// Lets the reconciler notice a rotated Secret without keeping the secret itself around.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        for part in [
            self.issuer.as_str(),
            self.client_id.as_str(),
            self.client_secret.as_str(),
            self.audience.as_deref().unwrap_or(""),
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0u8]);
        }
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("OIDC discovery failed: {0}")]
    Discovery(String),
    #[error("token request failed: {0}")]
    Request(String),
    #[error("token response has no access_token")]
    MalformedResponse,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

pub struct TokenCache {
    http: reqwest::Client,
    creds: OidcClientCredentials,
    // tokio Mutex: held across the await of a refresh so concurrent reconciles
    // share one token request instead of each hitting the IdP.
    state: Mutex<Option<CachedToken>>,
}

#[derive(Deserialize)]
struct Discovery {
    token_endpoint: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
}

impl TokenCache {
    pub fn new(http: reqwest::Client, creds: OidcClientCredentials) -> Self {
        Self {
            http,
            creds,
            state: Mutex::new(None),
        }
    }

    pub async fn bearer(&self) -> Result<String, TokenError> {
        let mut guard = self.state.lock().await;
        if let Some(token) = guard.as_ref()
            && token.expires_at > Instant::now() + REFRESH_MARGIN
        {
            return Ok(token.access_token.clone());
        }
        let fresh = self.fetch().await?;
        let access_token = fresh.access_token.clone();
        *guard = Some(fresh);
        Ok(access_token)
    }

    async fn fetch(&self) -> Result<CachedToken, TokenError> {
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.creds.issuer.trim_end_matches('/')
        );
        let discovery: Discovery = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| TokenError::Discovery(e.to_string()))?
            .json()
            .await
            .map_err(|e| TokenError::Discovery(e.to_string()))?;

        let mut form = vec![
            ("grant_type", "client_credentials"),
            ("client_id", self.creds.client_id.as_str()),
            ("client_secret", self.creds.client_secret.as_str()),
        ];
        if let Some(audience) = &self.creds.audience {
            form.push(("audience", audience.as_str()));
        }
        let response = self
            .http
            .post(&discovery.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| TokenError::Request(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(TokenError::Request(format!("HTTP {status}: {body}")));
        }
        let body: TokenResponse = response
            .json()
            .await
            .map_err(|e| TokenError::Request(e.to_string()))?;
        let access_token = body.access_token.ok_or(TokenError::MalformedResponse)?;
        let lifetime = body
            .expires_in
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_LIFETIME);
        Ok(CachedToken {
            access_token,
            expires_at: Instant::now() + lifetime,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(
            matches!(cache.bearer().await, Err(TokenError::Request(msg)) if msg.contains("401"))
        );
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
}
