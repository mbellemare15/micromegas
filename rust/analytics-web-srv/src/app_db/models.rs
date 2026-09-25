use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub use analytics_web_api::{
    CreateScreenRequest, Screen, UpdateScreenRequest, ValidationError, normalize_name,
    validate_folder_path, validate_name,
};

/// A folder in the screens hierarchy.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Folder {
    pub path: String,
}

/// A folder entry as returned by `GET /folders`, with derived counts.
#[derive(Debug, Clone, Serialize)]
pub struct FolderInfo {
    pub path: String,
    pub screen_count: i64,
    pub subfolder_count: i64,
}

/// Request to create a folder.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateFolderRequest {
    pub path: String,
}

/// Request to rename/move a folder.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFolderRequest {
    pub path: String,
    pub new_path: String,
}

/// A data source configuration stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DataSource {
    pub name: String,
    pub config: serde_json::Value,
    pub is_default: bool,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The config payload for a data source (deserialized from JSONB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceConfig {
    pub url: String,
}

/// Summary returned to non-admin users listing data sources.
#[derive(Debug, Clone, Serialize)]
pub struct DataSourceSummary {
    pub name: String,
    pub is_default: bool,
}

/// Request to create a new data source.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateDataSourceRequest {
    pub name: String,
    pub config: serde_json::Value,
    #[serde(default)]
    pub is_default: bool,
}

/// Request to update an existing data source.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDataSourceRequest {
    pub config: Option<serde_json::Value>,
    pub is_default: Option<bool>,
}

/// Validates a data source config JSONB value.
pub fn validate_data_source_config(
    config: &serde_json::Value,
) -> Result<DataSourceConfig, ValidationError> {
    let parsed: DataSourceConfig = serde_json::from_value(config.clone())
        .map_err(|e| ValidationError::new("INVALID_CONFIG", &format!("Invalid config: {e}")))?;
    if parsed.url.is_empty() {
        return Err(ValidationError::new(
            "MISSING_URL",
            "Config must include a non-empty 'url' field",
        ));
    }
    const ACCEPTED_URL_SCHEMES: &[&str] = &["http://", "https://", "grpc://", "grpc+tls://"];
    let url_lower = parsed.url.to_lowercase();
    if !ACCEPTED_URL_SCHEMES
        .iter()
        .any(|scheme| url_lower.starts_with(scheme))
    {
        return Err(ValidationError::new(
            "INVALID_URL",
            "URL must start with grpc://, grpc+tls://, http://, or https://",
        ));
    }
    Ok(parsed)
}

/// Expands `path` into itself and all of its ancestors, shortest first
/// (root-to-leaf), e.g. `"team/sub"` -> `["team", "team/sub"]`. When
/// `include_root` is true, the root (empty path) is prepended, e.g.
/// `expand_path_prefixes("team/dashboards", true)` ->
/// `["", "team", "team/dashboards"]`.
pub fn expand_path_prefixes(path: &str, include_root: bool) -> Vec<String> {
    let mut result = if include_root {
        vec![String::new()]
    } else {
        Vec::new()
    };
    if path.is_empty() {
        return result;
    }
    let mut cur = String::new();
    for segment in path.split('/') {
        cur = if cur.is_empty() {
            segment.to_string()
        } else {
            format!("{cur}/{segment}")
        };
        result.push(cur.clone());
    }
    result
}
