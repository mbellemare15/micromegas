//! Wire types and validation rules of the analytics-web-srv REST API.
//!
//! The server and its Rust clients (the Kubernetes operator) both depend on
//! this crate so a request built here is always one the server accepts.

pub mod screen_type;
pub mod screens;
pub mod validation;

pub use screen_type::{ParseScreenTypeError, ScreenType, ScreenTypeInfo};
pub use screens::{CreateScreenRequest, ErrorResponse, Screen, UpdateScreenRequest};
pub use validation::{ValidationError, normalize_name, validate_folder_path, validate_name};
