use serde::{Deserialize, Serialize};

pub type ApiResult<T> = Result<T, ApiError>;

/// Error returned to clients in `{"id":…, "error": {...}}`.
///
/// `code` is machine-readable and stable; `message` is for humans.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
    /// Malformed or missing parameters.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::new("not_found", format!("{what} not found"))
    }
    pub fn unknown_method(method: &str) -> Self {
        Self::new("unknown_method", format!("unknown method `{method}`"))
    }
    /// Operation not possible in the current state (e.g. agent busy, host offline).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }
    pub fn unauthorized() -> Self {
        Self::new("unauthorized", "missing or invalid token")
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new("unavailable", message)
    }
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::new("internal", message.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        Self::invalid(err.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Self::new("not_found", err.to_string()),
            std::io::ErrorKind::PermissionDenied => Self::new("forbidden", err.to_string()),
            _ => Self::internal(err),
        }
    }
}

/// Deserialize request params into `T`, mapping failures to `invalid_params`.
pub fn params<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> ApiResult<T> {
    let value = if value.is_null() { serde_json::Value::Object(Default::default()) } else { value };
    serde_json::from_value(value).map_err(|e| ApiError::invalid(e.to_string()))
}

/// Serialize a handler result into the response `result` value.
pub fn ok<T: Serialize>(value: T) -> ApiResult<serde_json::Value> {
    serde_json::to_value(value).map_err(ApiError::internal)
}
