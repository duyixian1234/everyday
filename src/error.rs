//! 统一错误类型 `AgentError`。
//!
//! JSON 模式下序列化为 PRD 规定的格式：
//! `{"error": "ErrorType", "message": "Details..."}`

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use thiserror::Error;

/// 全项目统一 Result 别名。
pub type Result<T> = std::result::Result<T, AgentError>;

/// 统一错误枚举。
///
/// 每个变体对应一个稳定的 `ErrorType` 字符串（见 [`AgentError::type_name`]），
/// 供 JSON 输出与程序化判断使用。
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

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("{0}")]
    Other(String),
}

impl AgentError {
    /// 返回稳定的错误类型名（PascalCase），用于 JSON 的 `error` 字段。
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
            Self::NotImplemented(_) => "NotImplemented",
            Self::Other(_) => "Other",
        }
    }

    /// 返回人类可读的错误详情，用于 JSON 的 `message` 字段。
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

// 自定义 Serialize：输出 {"error": "...", "message": "..."}
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
    fn json_error_format_matches_prd() {
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
            (AgentError::NotImplemented("x".into()), "NotImplemented"),
            (AgentError::Other("x".into()), "Other"),
        ];
        for (err, name) in cases {
            assert_eq!(err.type_name(), name);
        }
    }
}
