use std::path::Path;
use std::sync::PoisonError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("无效输入: {0}")]
    InvalidInput(String),
    #[error("IO 错误: {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{context}: {source}")]
    IoContext {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON 解析错误: {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("JSON 序列化失败: {source}")]
    JsonSerialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("TOML 解析错误: {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("锁获取失败: {0}")]
    Lock(String),
    #[error("MCP 校验失败: {0}")]
    McpValidation(String),
    #[error("{0}")]
    Message(String),
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("{zh} ({en})")]
    Localized {
        key: &'static str,
        zh: String,
        en: String,
    },
    #[error("数据库错误: {0}")]
    Database(String),
    #[error("OMO 配置文件不存在")]
    OmoConfigNotFound,
    #[error("所有供应商已熔断，无可用渠道")]
    AllProvidersCircuitOpen,
    #[error("未配置供应商")]
    NoProvidersConfigured,
    /// Structured failure for new commands (R-0 / BE-1).
    ///
    /// `code` is `SCREAMING_SNAKE_CASE`. `message` must not include filesystem
    /// paths or SQL. `details` is omitted from JSON when it is empty.
    #[error("{code}: {message}")]
    Coded {
        code: &'static str,
        message: String,
        details: Option<serde_json::Value>,
    },
}

/// Generic command error codes from the v7.3 error convention.
/// Command-specific codes stay at the call site and go through [`AppError::coded`].
pub const CODE_INTERNAL: &str = "INTERNAL";
pub const CODE_INVALID_ARGUMENT: &str = "INVALID_ARGUMENT";
pub const CODE_INVALID_AMOUNT: &str = "INVALID_AMOUNT";
pub const CODE_PRO_REQUIRED: &str = "PRO_REQUIRED";
pub const CODE_COMMUNITY_BUILD: &str = "COMMUNITY_BUILD";

fn details_payload_is_empty(details: &serde_json::Value) -> bool {
    details.is_null() || details.as_object().is_some_and(serde_json::Map::is_empty)
}

impl AppError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn json(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn toml(path: impl AsRef<Path>, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            key,
            zh: zh.into(),
            en: en.into(),
        }
    }

    /// Coded command error without `details`.
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self::Coded {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Coded command error. Null and empty-object `details` are stored as absent
    /// so serialization omits the field.
    pub fn coded_with_details(
        code: &'static str,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        let details = if details_payload_is_empty(&details) {
            None
        } else {
            Some(details)
        };
        Self::Coded {
            code,
            message: message.into(),
            details,
        }
    }

    pub fn invalid_argument(message: impl Into<String>, field: &str) -> Self {
        Self::coded_with_details(
            CODE_INVALID_ARGUMENT,
            message,
            serde_json::json!({ "field": field }),
        )
    }

    pub fn invalid_amount(field: &str) -> Self {
        Self::coded_with_details(
            CODE_INVALID_AMOUNT,
            "Invalid amount.",
            serde_json::json!({ "field": field }),
        )
    }

    /// Internal failure. `message` must not include a path or SQL text.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::coded(CODE_INTERNAL, message)
    }

    pub fn pro_required(message: impl Into<String>) -> Self {
        Self::coded(CODE_PRO_REQUIRED, message)
    }

    pub fn community_build(message: impl Into<String>) -> Self {
        Self::coded(CODE_COMMUNITY_BUILD, message)
    }

    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Coded { code, .. } => Some(*code),
            _ => None,
        }
    }
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(err: PoisonError<T>) -> Self {
        Self::Lock(err.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Coded {
                code,
                message,
                details,
            } => {
                let mut payload = serde_json::json!({
                    "code": code,
                    "message": message,
                });
                if let Some(details) = details {
                    if !details_payload_is_empty(details) {
                        payload["details"] = details.clone();
                    }
                }
                payload.serialize(serializer)
            }
            other => serializer.serialize_str(&other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailSerialize;

    impl serde::Serialize for FailSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("nope"))
        }
    }

    fn assert_legacy_json_string(err: &AppError) {
        let value = serde_json::to_value(err).expect("serialize");
        assert!(
            value.is_string(),
            "legacy variant must serialize as a JSON string, got {value}"
        );
        assert_eq!(value, serde_json::Value::String(err.to_string()));
    }

    /// Names every variant. Adding a variant without updating the snapshot
    /// test fails to compile.
    fn serialization_class(err: &AppError) -> &'static str {
        match err {
            AppError::Config(_)
            | AppError::InvalidInput(_)
            | AppError::Io { .. }
            | AppError::IoContext { .. }
            | AppError::Json { .. }
            | AppError::JsonSerialize { .. }
            | AppError::Toml { .. }
            | AppError::Lock(_)
            | AppError::McpValidation(_)
            | AppError::Message(_)
            | AppError::HttpStatus { .. }
            | AppError::Localized { .. }
            | AppError::Database(_)
            | AppError::OmoConfigNotFound
            | AppError::AllProvidersCircuitOpen
            | AppError::NoProvidersConfigured => "legacy-string",
            AppError::Coded { details, .. } => {
                if details.as_ref().is_none_or(details_payload_is_empty) {
                    "coded-no-details"
                } else {
                    "coded-with-details"
                }
            }
        }
    }

    #[test]
    fn every_legacy_variant_serializes_as_json_string() {
        let samples = [
            AppError::Config("bad".to_string()),
            AppError::InvalidInput("bad".to_string()),
            AppError::io("/tmp/demo", std::io::Error::other("disk")),
            AppError::IoContext {
                context: "read settings".to_string(),
                source: std::io::Error::other("disk"),
            },
            AppError::json(
                "/tmp/demo.json",
                serde_json::from_str::<u32>("{").expect_err("invalid json"),
            ),
            AppError::JsonSerialize {
                source: serde_json::to_string(&FailSerialize).expect_err("serialize"),
            },
            AppError::toml(
                "/tmp/demo.toml",
                toml::from_str::<toml::Value>("[").expect_err("invalid toml"),
            ),
            AppError::Lock("poisoned".to_string()),
            AppError::McpValidation("bad".to_string()),
            AppError::Message("hello".to_string()),
            AppError::HttpStatus {
                status: 503,
                body: "no".to_string(),
            },
            AppError::localized("demo.key", "中文", "English"),
            AppError::Database("down".to_string()),
            AppError::OmoConfigNotFound,
            AppError::AllProvidersCircuitOpen,
            AppError::NoProvidersConfigured,
        ];

        let expected = [
            "配置错误: bad",
            "无效输入: bad",
            "IO 错误: /tmp/demo: disk",
            "read settings: disk",
            "JSON 解析错误: /tmp/demo.json: invalid type: map, expected u32 at line 1 column 0",
            "JSON 序列化失败: nope",
            "TOML 解析错误: /tmp/demo.toml: TOML parse error at line 1, column 2\n  |\n1 | [\n  |  ^\nunquoted keys cannot be empty, expected letters, numbers, `-`, `_`\n",
            "锁获取失败: poisoned",
            "MCP 校验失败: bad",
            "hello",
            "HTTP 503: no",
            "中文 (English)",
            "数据库错误: down",
            "OMO 配置文件不存在",
            "所有供应商已熔断，无可用渠道",
            "未配置供应商",
        ];

        assert_eq!(samples.len(), expected.len());
        let mut mismatches = Vec::new();
        for (index, (err, display)) in samples.iter().zip(expected).enumerate() {
            assert_eq!(serialization_class(err), "legacy-string");
            if err.to_string() != display {
                mismatches.push(format!(
                    "#{index}\nactual: {:?}\nexpect: {display:?}",
                    err.to_string()
                ));
            }
            assert_legacy_json_string(err);
        }
        assert!(
            mismatches.is_empty(),
            "legacy display snapshots drifted:\n{}",
            mismatches.join("\n")
        );
    }

    #[test]
    fn coded_serializes_object_and_omits_empty_details() {
        let err =
            AppError::community_build("License activation is unavailable in a community build.");
        assert_eq!(serialization_class(&err), "coded-no-details");
        assert_eq!(
            serde_json::to_string(&err).expect("json"),
            r#"{"code":"COMMUNITY_BUILD","message":"License activation is unavailable in a community build."}"#
        );

        let null_details =
            AppError::coded_with_details(CODE_INTERNAL, "failed", serde_json::Value::Null);
        assert_eq!(serialization_class(&null_details), "coded-no-details");
        assert_eq!(
            serde_json::to_string(&null_details).expect("json"),
            r#"{"code":"INTERNAL","message":"failed"}"#
        );

        let empty_object =
            AppError::coded_with_details(CODE_INTERNAL, "failed", serde_json::json!({}));
        assert_eq!(serialization_class(&empty_object), "coded-no-details");
        assert!(empty_object.code() == Some(CODE_INTERNAL));

        let amount = AppError::invalid_amount("monthlyBudgetUsd");
        assert_eq!(serialization_class(&amount), "coded-with-details");
        assert_eq!(
            serde_json::to_string(&amount).expect("json"),
            r#"{"code":"INVALID_AMOUNT","message":"Invalid amount.","details":{"field":"monthlyBudgetUsd"}}"#
        );

        let argument = AppError::invalid_argument("token must not be empty.", "token");
        assert_eq!(
            serde_json::to_value(&argument).expect("json"),
            serde_json::json!({
                "code": "INVALID_ARGUMENT",
                "message": "token must not be empty.",
                "details": { "field": "token" }
            })
        );
    }
}
