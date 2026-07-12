//! Unified error type `AgentError`.
//!
//! In JSON mode it serializes to the format mandated by
//! [agents.md](../../agents.md):
//! `{"error": "ErrorType", "message": "Details..."}`

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use thiserror::Error;

/// Project-wide unified `Result` alias.
pub type Result<T> = std::result::Result<T, AgentError>;

/// Unified error enum.
///
/// Each variant maps to a stable `ErrorType` string (see
/// [`AgentError::type_name`]), used by JSON output and programmatic checks.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("config error: {0}")]
    Config(String),

    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("module not found: {0}")]
    ModuleNotFound(String),

    #[error("unknown action: {0}")]
    UnknownAction(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("{0}")]
    Other(String),
}

impl AgentError {
    /// Return the stable error type name (PascalCase), used for the JSON `error` field.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Config(_) => "ConfigError",
            Self::AccountNotFound(_) => "AccountNotFound",
            Self::Auth(_) => "AuthError",
            Self::Network(_) => "NetworkError",
            Self::Io(_) => "IoError",
            Self::ModuleNotFound(_) => "ModuleNotFound",
            Self::UnknownAction(_) => "UnknownAction",
            Self::InvalidArgument(_) => "InvalidArgument",
            Self::PermissionDenied(_) => "PermissionDenied",
            Self::Other(_) => "Other",
        }
    }

    /// Return a human-readable error detail, used for the JSON `message` field.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl From<std::io::Error> for AgentError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            Self::PermissionDenied(err.to_string())
        } else {
            Self::Io(err.to_string())
        }
    }
}

impl From<toml::de::Error> for AgentError {
    fn from(err: toml::de::Error) -> Self {
        Self::Config(format!("failed to parse config: {err}"))
    }
}

impl From<serde_json::Error> for AgentError {
    fn from(err: serde_json::Error) -> Self {
        Self::Other(format!("json error: {err}"))
    }
}

impl From<anyhow::Error> for AgentError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<sqlx::Error> for AgentError {
    fn from(err: sqlx::Error) -> Self {
        Self::Other(format!("sqlite error: {err}"))
    }
}

// Custom Serialize: emit `{"error": "...", "message": "..."}`.
impl Serialize for AgentError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AgentError", 2)?;
        s.serialize_field("error", self.type_name())?;
        s.serialize_field("message", &self.message())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_error_format_matches_spec() {
        let err = AgentError::AccountNotFound("work".into());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"], "AccountNotFound");
        assert_eq!(json["message"], "account not found: work");
    }

    #[test]
    fn io_permission_denied_maps_correctly() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let agent_err: AgentError = io_err.into();
        assert_eq!(agent_err.type_name(), "PermissionDenied");
    }

    #[test]
    fn type_names_are_stable() {
        let cases = [
            (AgentError::Config("x".into()), "ConfigError"),
            (AgentError::Network("x".into()), "NetworkError"),
            (AgentError::Other("x".into()), "Other"),
        ];
        for (err, name) in cases {
            assert_eq!(err.type_name(), name);
        }
    }
}
