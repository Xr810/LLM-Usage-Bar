//! 请求上下文模块
//!
//! 提供请求生命周期的上下文管理，封装通用初始化逻辑

use crate::app_config::AppType;
use crate::credentials::{
    CredentialExposureGuardSet as CredentialExposureGuard, ResolvedBindingCredential, SecretString,
};
#[cfg(test)]
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::{
    extract_session_id,
    forwarder::RequestForwarder,
    provider_router::BindingPricingOverride,
    server::ProxyState,
    types::{AppProxyConfig, CopilotOptimizerConfig, OptimizerConfig, RectifierConfig},
    ProxyError,
};
use crate::usage::ingestion::FrozenUsageProviderContext;
use axum::http::HeaderMap;
use std::time::Instant;

/// 流式超时配置
#[derive(Debug, Clone, Copy)]
pub struct StreamingTimeoutConfig {
    /// 首字节超时（秒），0 表示禁用
    pub first_byte_timeout: u64,
    /// 静默期超时（秒），0 表示禁用
    pub idle_timeout: u64,
}

/// 请求上下文
///
/// 贯穿整个请求生命周期，包含：
/// - 计时信息
/// - 应用级代理配置（per-app）
/// - 选中的 Provider 列表（用于故障转移）
/// - 请求模型名称
/// - 日志标签
/// - Session ID（用于日志关联）
pub struct RequestContext {
    /// 请求开始时间
    pub start_time: Instant,
    /// 应用级代理配置（per-app，包含重试次数和超时配置）
    pub app_config: AppProxyConfig,
    /// 选中的 Provider（故障转移链的第一个）
    pub provider: Provider,
    /// Binding identity frozen at the same lookup boundary as the route and
    /// protected credential. This remains available after the secret is moved
    /// into and dropped by the forwarder.
    pub binding_id: String,
    /// Agent identity frozen together with the binding, Provider route, and
    /// protected key at request resolution time.
    pub agent_module_id: String,
    /// v13 全局 Provider ID；与兼容运行时 `provider.id` 明确分离。
    pub usage_provider_id: String,
    /// Provider grouping/protocol frozen at the binding lookup linearization
    /// point so asynchronous ingestion cannot observe later Provider edits.
    pub frozen_usage_provider_context: FrozenUsageProviderContext,
    /// Explicit v12 migration identity used only for compatibility log rows.
    /// Unlinked v13 Providers fall back to their own runtime identity without
    /// gaining access to a same-ID v12 pricing record.
    pub legacy_log_provider_id: String,
    /// Safe pricing fields frozen from an explicit legacy migration snapshot.
    pub pricing_override: BindingPricingOverride,
    /// Non-clone protected credential. Handlers move this into the forwarder so
    /// it is dropped immediately after the upstream request attempt finishes.
    binding_credential: Option<ResolvedBindingCredential>,
    /// Request-lifetime response guard retained after the forwarding copy is
    /// consumed. Its protected zeroized key copy enables exact prefix-safe
    /// egress inspection and is dropped with the request context.
    credential_exposure_guard: CredentialExposureGuard,
    /// 完整的 Provider 列表（用于故障转移）
    providers: Vec<Provider>,
    /// 请求中的模型名称
    pub request_model: String,
    /// 实际发往上游的模型名（路由接管/模型映射后的真值，forward 成功后回填）。
    ///
    /// usage 归因的兜底顺序：上游响应回显 → outbound_model → request_model。
    /// 不能直接用 request_model 兜底：接管场景下它是映射前的客户端别名。
    pub outbound_model: Option<String>,
    /// 日志标签（如 "Claude"、"Codex"、"Gemini"）
    pub tag: &'static str,
    /// 应用类型字符串（如 "claude"、"codex"、"gemini"）
    pub app_type_str: &'static str,
    /// 应用类型（预留，目前通过 app_type_str 使用）
    #[allow(dead_code)]
    pub app_type: AppType,
    /// Session ID（从客户端请求提取或新生成）
    pub session_id: String,
    /// Session ID 是否由客户端提供。生成的 UUID 不能作为上游缓存 key，否则每个请求都会换 key。
    pub session_client_provided: bool,
    /// 整流器配置
    pub rectifier_config: RectifierConfig,
    /// 优化器配置
    pub optimizer_config: OptimizerConfig,
    /// Copilot 优化器配置
    pub copilot_optimizer_config: CopilotOptimizerConfig,
}

impl RequestContext {
    /// 创建请求上下文
    ///
    /// # Arguments
    /// * `state` - 代理服务器状态
    /// * `body` - 请求体 JSON
    /// * `headers` - 请求头（用于提取 Session ID）
    /// * `app_type` - 应用类型
    /// * `tag` - 日志标签
    /// * `app_type_str` - 应用类型字符串
    ///
    /// # Errors
    /// 返回 `ProxyError` 如果 Provider 选择失败
    pub async fn new(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        binding_key: SecretString,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
    ) -> Result<Self, ProxyError> {
        let start_time = Instant::now();

        // 从数据库读取应用级代理配置（per-app）
        let app_config = state
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        // 从数据库读取整流器配置
        let rectifier_config = state.db.get_rectifier_config().unwrap_or_default();
        let optimizer_config = state.db.get_optimizer_config().unwrap_or_default();
        let copilot_optimizer_config = state.db.get_copilot_optimizer_config().unwrap_or_default();

        // The protected binding lookup is the request-path linearization point.
        // Every failure is deliberately collapsed to one payload-free local
        // authorization error so credential/store/binding state is not exposed.
        let binding_credential = state
            .binding_credential_service
            .resolve_binding_api_key(binding_key)
            .await
            .map_err(|_| ProxyError::BindingAuthorizationFailed)?;
        if binding_credential.route_protocol() != canonical_route_protocol(app_type_str) {
            return Err(ProxyError::BindingAuthorizationFailed);
        }
        let credential_exposure_guard = binding_credential.exposure_guard();
        let request_model = credential_exposure_guard
            .redact_or(
                body.get("model")
                    .and_then(|model| model.as_str())
                    .unwrap_or("unknown"),
                "unknown",
            )
            .to_string();
        let session_result = extract_session_id(headers, body, app_type_str);
        let session_was_secret = credential_exposure_guard.contains(&session_result.session_id);
        let session_id = if session_was_secret {
            uuid::Uuid::new_v4().to_string()
        } else {
            session_result.session_id.clone()
        };
        let session_client_provided = session_result.client_provided && !session_was_secret;
        log::debug!(
            "[{tag}] Session resolved (source={:?}, client_provided={session_client_provided}); value omitted",
            session_result.source
        );
        let ownership = binding_credential.frozen_ownership();
        let frozen_usage_provider_context = FrozenUsageProviderContext {
            product_group_id: binding_credential.product_group_id().to_string(),
            route_app_type: binding_credential.route_protocol().to_string(),
        };
        let binding_id = ownership.binding_id;
        let agent_module_id = ownership.agent_module_id;
        let usage_provider_id = ownership.provider_id;
        let provider = binding_credential.runtime_provider().clone();
        let legacy_log_provider_id = binding_credential
            .legacy_pricing_provider_id()
            .unwrap_or(provider.id.as_str())
            .to_string();
        let pricing_override = binding_credential.pricing_override().clone();
        let providers = vec![provider.clone()];

        log::debug!(
            "[{tag}] Binding resolved: agent={agent_module_id}, provider={usage_provider_id}; model/session values omitted"
        );

        Ok(Self {
            start_time,
            app_config,
            provider,
            binding_id,
            agent_module_id,
            usage_provider_id,
            frozen_usage_provider_context,
            legacy_log_provider_id,
            pricing_override,
            binding_credential: Some(binding_credential),
            credential_exposure_guard,
            providers,
            request_model,
            outbound_model: None,
            tag,
            app_type_str,
            app_type,
            session_id,
            session_client_provided,
            rectifier_config,
            optimizer_config,
            copilot_optimizer_config,
        })
    }

    pub(crate) fn credential_exposure_guard(&self) -> &CredentialExposureGuard {
        &self.credential_exposure_guard
    }

    /// Move the verified protected credential into the forwarder. The
    /// credential is non-clone, so no response/logging task can retain it.
    pub(crate) fn take_binding_credential(
        &mut self,
    ) -> Result<ResolvedBindingCredential, ProxyError> {
        self.binding_credential
            .take()
            .ok_or(ProxyError::BindingAuthorizationFailed)
    }

    /// 从 URI 提取模型名称（Gemini 专用）
    ///
    /// Gemini API 的模型名称在 URI 中，格式如：
    /// `/v1beta/models/gemini-pro:generateContent`
    pub fn with_model_from_uri(mut self, uri: &axum::http::Uri) -> Self {
        // 用 path() 而不是 path_and_query()：模型名必须从路径段中解析，
        // 否则 GET /v1beta/models/<id>?key=... 会把 query 拼到 request_model 上。
        let endpoint = uri.path();

        let model =
            extract_gemini_model_from_path(endpoint).unwrap_or_else(|| "unknown".to_string());
        self.request_model = self
            .credential_exposure_guard
            .redact_or(&model, "unknown")
            .to_string();

        self
    }

    /// 创建 RequestForwarder
    ///
    /// Preserve the existing timeout and protocol-adapter configuration while
    /// forcing the provider-attempt count to one.
    pub fn create_forwarder(&self, state: &ProxyState) -> RequestForwarder {
        RequestForwarder::new(
            self.app_config.non_streaming_timeout as u64,
            state.status.clone(),
            state.current_providers.clone(),
            state.gemini_shadow.clone(),
            state.codex_chat_history.clone(),
            self.binding_id.clone(),
            state.app_handle.clone(),
            self.session_id.clone(),
            self.session_client_provided,
            self.app_config.streaming_first_byte_timeout as u64,
            self.app_config.streaming_idle_timeout as u64,
            self.rectifier_config.clone(),
            self.optimizer_config.clone(),
            self.copilot_optimizer_config.clone(),
            self.credential_exposure_guard.clone(),
        )
        .without_reactive_retries()
    }

    /// 获取 Provider 列表（用于故障转移）
    ///
    /// Returns the single provider selected from the static binding.
    pub fn get_providers(&self) -> Vec<Provider> {
        self.providers.clone()
    }

    /// 计算请求延迟（毫秒）
    #[inline]
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// 获取流式超时配置
    ///
    /// 配置生效规则：
    /// - 故障转移开启：返回配置的值（0 表示禁用超时检查）
    /// - 故障转移关闭：返回 0（禁用超时检查）
    #[inline]
    pub fn streaming_timeout_config(&self) -> StreamingTimeoutConfig {
        if self.app_config.auto_failover_enabled {
            // 故障转移开启：使用配置的值（0 = 禁用超时）
            StreamingTimeoutConfig {
                first_byte_timeout: self.app_config.streaming_first_byte_timeout as u64,
                idle_timeout: self.app_config.streaming_idle_timeout as u64,
            }
        } else {
            // 故障转移关闭：禁用流式超时检查
            StreamingTimeoutConfig {
                first_byte_timeout: 0,
                idle_timeout: 0,
            }
        }
    }
}

/// Pull the Gemini model name out of an API path.
///
/// Accepts forms like `/v1beta/models/gemini-pro:generateContent`,
/// `/v1/models/gemini-1.5-flash`, `gemini/v1beta/models/<model>:streamGenerateContent`.
/// Returns `None` when no `models/<name>` segment is present.
pub(crate) fn extract_gemini_model_from_path(endpoint: &str) -> Option<String> {
    let segments: Vec<&str> = endpoint.split('/').collect();
    segments
        .iter()
        .position(|s| *s == "models")
        .and_then(|i| segments.get(i + 1).copied())
        // 防御性裁剪：即便调用方传入带 ? 或 :action 的字符串，也只保留 model id 本身
        .map(|s| s.split('?').next().unwrap_or(s))
        .map(|s| s.split(':').next().unwrap_or(s))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Persisted route protocol identity is exact. Claude Desktop has its own
/// migrated Provider rows and must not consume an ordinary Claude binding (or
/// vice versa), even though both ultimately speak an Anthropic wire format.
pub(crate) fn canonical_route_protocol(app_type: &str) -> &str {
    app_type
}

/// Convert route-selection errors from `ProviderRouter::select_bound_provider` into typed
/// `ProxyError` variants so callers can respond with consistent HTTP status codes.
#[cfg(test)]
pub(crate) fn map_route_selection_error(error: AppError) -> ProxyError {
    match error {
        AppError::Message(message) => {
            if let Some(value) = message.strip_prefix("route not bound: ") {
                ProxyError::RouteNotBound(value.to_string())
            } else if let Some(value) = message.strip_prefix("route provider disabled: ") {
                ProxyError::RouteProviderDisabled(value.to_string())
            } else if let Some(value) = message.strip_prefix("route provider is not metered: ") {
                ProxyError::RouteProviderNotMetered(value.to_string())
            } else if let Some(value) = message.strip_prefix("route config incomplete: ") {
                ProxyError::RouteConfigIncomplete(value.to_string())
            } else {
                ProxyError::Internal(message)
            }
        }
        _ => ProxyError::Internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_route_protocol, extract_gemini_model_from_path, map_route_selection_error,
    };
    use crate::error::AppError;
    use crate::proxy::ProxyError;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn route_selection_failures_are_local_service_unavailable_errors() {
        let cases = [
            (
                "route not bound: claude",
                ProxyError::RouteNotBound("claude".to_string()),
            ),
            (
                "route provider disabled: disabled-id",
                ProxyError::RouteProviderDisabled("disabled-id".to_string()),
            ),
            (
                "route provider is not metered: subscription-id",
                ProxyError::RouteProviderNotMetered("subscription-id".to_string()),
            ),
            (
                "route config incomplete: incomplete-id",
                ProxyError::RouteConfigIncomplete("incomplete-id".to_string()),
            ),
        ];

        for (message, expected) in cases {
            let error = map_route_selection_error(AppError::Message(message.to_string()));
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected)
            );
            assert_eq!(
                error.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
    }

    #[test]
    fn claude_desktop_keeps_its_exact_route_identity() {
        assert_eq!(canonical_route_protocol("claude-desktop"), "claude-desktop");
        assert_eq!(canonical_route_protocol("codex"), "codex");
        assert_eq!(canonical_route_protocol("gemini"), "gemini");
    }

    #[test]
    fn extract_model_with_action() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro:generateContent").as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_with_dotted_version() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-1.5-flash:streamGenerateContent")
                .as_deref(),
            Some("gemini-1.5-flash"),
        );
    }

    #[test]
    fn extract_model_without_action() {
        assert_eq!(
            extract_gemini_model_from_path("/v1/models/gemini-1.5-pro").as_deref(),
            Some("gemini-1.5-pro"),
        );
    }

    #[test]
    fn extract_model_with_proxy_prefix() {
        assert_eq!(
            extract_gemini_model_from_path("/gemini/v1beta/models/gemini-2.0-flash:countTokens")
                .as_deref(),
            Some("gemini-2.0-flash"),
        );
    }

    #[test]
    fn extract_model_with_query_string() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro:generateContent?key=abc")
                .as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_missing_segment() {
        assert_eq!(extract_gemini_model_from_path("/v1beta/operations"), None);
    }

    #[test]
    fn extract_model_trailing_models_segment() {
        // `/v1beta/models` (list endpoint) has no following segment → None.
        assert_eq!(extract_gemini_model_from_path("/v1beta/models"), None);
    }

    #[test]
    fn extract_model_get_with_query_only() {
        // GET /v1beta/models/<id>?key=... 无 action verb，仅靠 ':' 拆分会把 query 带进 model 名。
        // 修复后应该把 query 剥掉。
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro?key=abc").as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_get_with_proxy_prefix_and_query() {
        assert_eq!(
            extract_gemini_model_from_path("/gemini/v1beta/models/gemini-2.0-flash?key=abc")
                .as_deref(),
            Some("gemini-2.0-flash"),
        );
    }
}
