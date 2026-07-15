use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

use crate::credentials::CredentialExposureGuardSet as CredentialExposureGuard;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("服务器已在运行")]
    AlreadyRunning,

    #[error("服务器未运行")]
    NotRunning,

    #[error("地址绑定失败: {0}")]
    BindFailed(String),

    #[error("停止超时")]
    StopTimeout,

    #[error("停止失败: {0}")]
    StopFailed(String),

    #[error("请求转发失败: {0}")]
    ForwardFailed(String),

    #[error("无可用的Provider")]
    NoAvailableProvider,

    #[allow(dead_code)] // v12 compatibility; removed after the stable-v13 gate.
    #[error("所有供应商已熔断，无可用渠道")]
    AllProvidersCircuitOpen,

    #[allow(dead_code)] // v12 compatibility; static routes use RouteNotBound.
    #[error("未配置供应商")]
    NoProvidersConfigured,

    #[error("路由未绑定: {0}")]
    RouteNotBound(String),

    #[error("路由供应商已禁用: {0}")]
    RouteProviderDisabled(String),

    #[error("路由供应商非按量计费: {0}")]
    RouteProviderNotMetered(String),

    #[error("路由配置不完整: {0}")]
    RouteConfigIncomplete(String),

    #[allow(dead_code)]
    #[error("Provider不健康: {0}")]
    ProviderUnhealthy(String),

    // The body remains available only inside the forwarder for protocol
    // rectification/classification. Display and public boundaries must never
    // render it because an upstream can echo the protected binding key.
    #[error("上游错误 (状态码 {status})")]
    UpstreamError { status: u16, body: Option<String> },

    /// A fixed, payload-free failure used when an upstream response reflects a
    /// protected binding credential or cannot be safely inspected.
    #[error("upstream response rejected")]
    UpstreamResponseRejected,

    #[error("超过最大重试次数")]
    MaxRetriesExceeded,

    #[error("数据库错误: {0}")]
    DatabaseError(String),

    #[error("配置错误: {0}")]
    ConfigError(String),

    #[allow(dead_code)]
    #[error("格式转换错误: {0}")]
    TransformError(String),

    #[allow(dead_code)]
    #[error("无效的请求: {0}")]
    InvalidRequest(String),

    #[error("超时: {0}")]
    Timeout(String),

    /// 流式响应空闲超时
    #[allow(dead_code)]
    #[error("流式响应空闲超时: {0}秒无数据")]
    StreamIdleTimeout(u64),

    /// 认证错误
    #[error("认证失败: {0}")]
    AuthError(String),

    /// Local binding credentials are deliberately indistinguishable: missing,
    /// unknown, disabled, archived, protocol-mismatched, and store failures all
    /// return this payload-free error without consulting an upstream.
    #[error("binding authorization failed")]
    BindingAuthorizationFailed,

    #[allow(dead_code)]
    #[error("内部错误: {0}")]
    Internal(String),
}

impl ProxyError {
    /// Preserve the error category/status while removing any string that
    /// contains the protected binding credential. This must run before an
    /// error reaches retry diagnostics, proxy status, persistence, or HTTP.
    pub(crate) fn redact_credential(self, guard: &CredentialExposureGuard) -> Self {
        fn redact(
            value: String,
            replacement: &'static str,
            guard: &CredentialExposureGuard,
        ) -> String {
            if guard.contains(&value) {
                replacement.to_string()
            } else {
                value
            }
        }

        match self {
            Self::BindFailed(value) => {
                Self::BindFailed(redact(value, "address binding failed", guard))
            }
            Self::StopFailed(value) => Self::StopFailed(redact(value, "proxy stop failed", guard)),
            Self::ForwardFailed(value) => {
                Self::ForwardFailed(redact(value, "request forwarding failed", guard))
            }
            Self::RouteNotBound(value) => {
                Self::RouteNotBound(redact(value, "route unavailable", guard))
            }
            Self::RouteProviderDisabled(value) => {
                Self::RouteProviderDisabled(redact(value, "route unavailable", guard))
            }
            Self::RouteProviderNotMetered(value) => {
                Self::RouteProviderNotMetered(redact(value, "route unavailable", guard))
            }
            Self::RouteConfigIncomplete(value) => {
                Self::RouteConfigIncomplete(redact(value, "route unavailable", guard))
            }
            Self::ProviderUnhealthy(value) => {
                Self::ProviderUnhealthy(redact(value, "provider unavailable", guard))
            }
            Self::UpstreamError { status, body } => Self::UpstreamError {
                status,
                body: body.filter(|value| !guard.contains(value)),
            },
            Self::DatabaseError(value) => {
                Self::DatabaseError(redact(value, "database operation failed", guard))
            }
            Self::ConfigError(value) => {
                Self::ConfigError(redact(value, "invalid proxy configuration", guard))
            }
            Self::TransformError(value) => Self::TransformError(redact(
                value,
                "request or upstream transformation failed",
                guard,
            )),
            Self::InvalidRequest(value) => {
                Self::InvalidRequest(redact(value, "invalid request", guard))
            }
            Self::Timeout(value) => Self::Timeout(redact(value, "upstream timeout", guard)),
            Self::AuthError(value) => {
                Self::AuthError(redact(value, "authentication failed", guard))
            }
            Self::Internal(value) => Self::Internal(redact(value, "internal proxy error", guard)),
            error => error,
        }
    }

    pub(crate) fn discard_upstream_body(&mut self) {
        if let ProxyError::UpstreamError { body, .. } = self {
            *body = None;
        }
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ProxyError::UpstreamError {
                status: upstream_status,
                ..
            } => {
                let http_status =
                    StatusCode::from_u16(*upstream_status).unwrap_or(StatusCode::BAD_GATEWAY);

                let error_body = json!({
                    "error": {
                        "message": format!("Upstream error (status {})", upstream_status),
                        "type": "upstream_error",
                    }
                });

                (http_status, error_body)
            }
            _ => {
                let (http_status, message) = match &self {
                    ProxyError::AlreadyRunning => (StatusCode::CONFLICT, self.to_string()),
                    ProxyError::NotRunning => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
                    ProxyError::BindFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopTimeout => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ForwardFailed(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
                    ProxyError::UpstreamResponseRejected => {
                        (StatusCode::BAD_GATEWAY, self.to_string())
                    }
                    ProxyError::NoAvailableProvider => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::AllProvidersCircuitOpen => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::NoProvidersConfigured => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::RouteNotBound(_)
                    | ProxyError::RouteProviderDisabled(_)
                    | ProxyError::RouteProviderNotMetered(_)
                    | ProxyError::RouteConfigIncomplete(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::ProviderUnhealthy(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::MaxRetriesExceeded => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::DatabaseError(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ConfigError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::TransformError(_) => {
                        (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
                    }
                    ProxyError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
                    ProxyError::StreamIdleTimeout(_) => {
                        (StatusCode::GATEWAY_TIMEOUT, self.to_string())
                    }
                    ProxyError::AuthError(_) | ProxyError::BindingAuthorizationFailed => {
                        (StatusCode::UNAUTHORIZED, self.to_string())
                    }
                    ProxyError::Internal(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::UpstreamError { .. } => unreachable!(),
                };

                let error_body = json!({
                    "error": {
                        "message": message,
                        "type": "proxy_error",
                    }
                });

                (http_status, error_body)
            }
        };

        (status, Json(body)).into_response()
    }
}

/// 错误分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// 可重试错误（网络问题、5xx）
    Retryable, // 网络超时、5xx 错误
    /// 不可重试错误（4xx、认证失败）
    NonRetryable, // 认证失败、参数错误、4xx 错误
    #[allow(dead_code)]
    ClientAbort, // 客户端主动中断
}

/// 判断错误是否可重试
#[allow(dead_code)]
pub fn categorize_error(error: &reqwest::Error) -> ErrorCategory {
    if error.is_timeout() || error.is_connect() {
        return ErrorCategory::Retryable;
    }

    if let Some(status) = error.status() {
        if status.is_server_error() {
            ErrorCategory::Retryable
        } else if status.is_client_error() {
            ErrorCategory::NonRetryable
        } else {
            ErrorCategory::Retryable
        }
    } else {
        ErrorCategory::Retryable
    }
}

#[cfg(test)]
mod tests {
    use super::ProxyError;
    use crate::credentials::CredentialExposureGuardSet as CredentialExposureGuard;

    #[test]
    fn credential_redaction_preserves_error_kind_without_rendering_secret() {
        const SECRET: &str = "protected-binding-error-sentinel";
        let guard = CredentialExposureGuard::from_secret(SECRET.as_bytes());

        let error = ProxyError::TransformError(format!(
            "Gemini Native only supports base64 image sources, got `{SECRET}`"
        ))
        .redact_credential(&guard);

        assert!(matches!(error, ProxyError::TransformError(_)));
        assert!(!error.to_string().contains(SECRET));
        assert_eq!(
            error.to_string(),
            "格式转换错误: request or upstream transformation failed"
        );
    }

    #[test]
    fn credential_redaction_removes_secret_from_upstream_body() {
        const SECRET: &str = "protected-upstream-body-sentinel";
        let guard = CredentialExposureGuard::from_secret(SECRET.as_bytes());

        let error = ProxyError::UpstreamError {
            status: 401,
            body: Some(format!(r#"{{"message":"invalid {SECRET}"}}"#)),
        }
        .redact_credential(&guard);

        assert!(matches!(
            error,
            ProxyError::UpstreamError {
                status: 401,
                body: None
            }
        ));
    }
}
