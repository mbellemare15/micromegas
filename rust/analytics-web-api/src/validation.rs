//! Name and folder-path rules enforced by analytics-web-srv. Clients validate
//! with the same code so a rejected request is caught before it is sent.

use serde::{Deserialize, Serialize};

/// Reserved names that cannot be used.
const RESERVED_NAMES: &[&str] = &["new"];

/// Validation error for screen names and folder paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

/// Normalizes a name for URL usage.
///
/// - Converts to lowercase
/// - Replaces spaces with hyphens
/// - Removes invalid characters
/// - Collapses consecutive hyphens
pub fn normalize_name(name: &str) -> String {
    let normalized: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' { '-' } else { c })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();

    // Collapse consecutive hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in normalized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading and trailing hyphens
    result.trim_matches('-').to_string()
}

/// Shared core for `validate_name` and the folder-segment validator.
///
/// - `min_length`: minimum character count (3 for screen names, 1 for folder segments).
/// - `check_reserved`: whether `RESERVED_NAMES` is enforced (screen names only — `"new"`
///   collides with the `/screen/new` route, which doesn't apply to folders).
/// - `enforce_boundaries`: whether the name must start with a letter and end with a
///   letter or digit (screen names only — folder segments may start/end with a digit,
///   e.g. a year like `2025`).
fn validate_name_core(
    name: &str,
    min_length: usize,
    check_reserved: bool,
    enforce_boundaries: bool,
) -> Result<(), ValidationError> {
    // Check length
    if name.len() < min_length {
        return Err(ValidationError::new(
            "NAME_TOO_SHORT",
            &format!("Name must be at least {min_length} characters"),
        ));
    }
    if name.len() > 100 {
        return Err(ValidationError::new(
            "NAME_TOO_LONG",
            "Name must be at most 100 characters",
        ));
    }

    // Check reserved names
    if check_reserved && RESERVED_NAMES.contains(&name) {
        return Err(ValidationError::new(
            "RESERVED_NAME",
            "This name is reserved",
        ));
    }

    // Check characters
    let chars: Vec<char> = name.chars().collect();

    if enforce_boundaries {
        // Must start with a letter
        if !chars.first().is_some_and(|c| c.is_ascii_lowercase()) {
            return Err(ValidationError::new(
                "INVALID_START",
                "Name must start with a lowercase letter",
            ));
        }

        // Must end with a letter or number
        if !chars
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err(ValidationError::new(
                "INVALID_END",
                "Name must end with a letter or number",
            ));
        }
    }

    // Check all characters are valid and no consecutive hyphens
    let mut prev_hyphen = false;
    for c in &chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && *c != '-' {
            return Err(ValidationError::new(
                "INVALID_CHARACTER",
                "Name can only contain lowercase letters, numbers, and hyphens",
            ));
        }
        if *c == '-' {
            if prev_hyphen {
                return Err(ValidationError::new(
                    "CONSECUTIVE_HYPHENS",
                    "Name cannot contain consecutive hyphens",
                ));
            }
            prev_hyphen = true;
        } else {
            prev_hyphen = false;
        }
    }

    Ok(())
}

/// Validates a name according to the rules:
/// - 3-100 characters
/// - Lowercase letters, numbers, and hyphens only
/// - Must start with a letter
/// - Must end with a letter or number
/// - No consecutive hyphens
/// - Not a reserved name
pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    validate_name_core(name, 3, true, true)
}

/// Maximum length of a composed folder path (matches the `VARCHAR(1024)` columns).
const MAX_FOLDER_PATH_LENGTH: usize = 1024;

/// Validates a single folder path segment:
/// - 1-100 characters
/// - Lowercase letters, numbers, and hyphens only
/// - No consecutive hyphens
/// - No reserved-word check (`"new"` is a valid folder name)
/// - No start/end-character restriction (a segment may start or end with a digit)
fn validate_folder_segment(segment: &str) -> Result<(), ValidationError> {
    validate_name_core(segment, 1, false, false)
}

/// Validates a folder path (`/`-delimited, no leading/trailing slash, `""` = root).
///
/// An empty path is always a valid root and is returned as-is without being split
/// on `/` — `"".split('/')` yields one empty segment, which would otherwise fail the
/// minimum-length check. A non-empty path is split on `/` and each segment is
/// validated individually, then the total composed length is checked against the
/// `VARCHAR(1024)` column limit.
pub fn validate_folder_path(path: &str) -> Result<(), ValidationError> {
    if path.is_empty() {
        return Ok(());
    }
    for segment in path.split('/') {
        validate_folder_segment(segment)?;
    }
    if path.len() > MAX_FOLDER_PATH_LENGTH {
        return Err(ValidationError::new(
            "PATH_TOO_LONG",
            &format!("Folder path must be at most {MAX_FOLDER_PATH_LENGTH} characters"),
        ));
    }
    Ok(())
}
