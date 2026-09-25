use crate::validation::ValidationError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A screen as returned by `GET /api/screens/{name}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct Screen {
    pub name: String,
    pub screen_type: String,
    pub config: serde_json::Value,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub managed_by: Option<String>,
    pub folder_path: String,
}

/// Body of `POST /api/screens`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateScreenRequest {
    pub name: String,
    pub screen_type: String,
    pub config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,
    #[serde(default)]
    pub folder_path: String,
}

/// Body of `PUT /api/screens/{name}`. Every field is optional; the server keeps
/// the current value for anything omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateScreenRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
}

/// Error body returned by every `/api/screens` route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl From<ValidationError> for ErrorResponse {
    fn from(err: ValidationError) -> Self {
        Self {
            code: err.code,
            message: err.message,
        }
    }
}
