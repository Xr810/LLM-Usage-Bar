//! 供应商路由器模块
//!
//! 负责选择和管理代理目标供应商，实现智能故障转移

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};
use crate::proxy::providers::get_adapter;
use crate::usage::domain::BillingKind;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 供应商路由器
pub struct ProviderRouter {
    /// 数据库连接
    db: Arc<Database>,
    /// 熔断器管理器 - key 格式: "app_type:provider_id"
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
}

#[derive(Debug, Clone)]
pub struct BoundProvider {
    /// Global v13 identity used by usage_events and quota/source ownership.
    pub usage_provider_id: String,
    /// Transitional Provider used by the existing protocol adapters.
    pub provider: Provider,
}

impl ProviderRouter {
    /// 创建新的供应商路由器
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Resolve the single provider statically bound to `protocol`.
    ///
    /// Bindings and provider records are deliberately read on every call so a
    /// stale binding cannot continue routing after its target is disabled or
    /// changed to subscription billing.
    pub async fn select_bound_provider(&self, protocol: &str) -> Result<Provider, AppError> {
        self.select_bound_route(protocol)
            .await
            .map(|bound| bound.provider)
    }

    pub async fn select_bound_route(&self, protocol: &str) -> Result<BoundProvider, AppError> {
        let binding = self
            .db
            .get_route_bindings()?
            .into_iter()
            .find(|binding| binding.protocol == protocol)
            .ok_or_else(|| AppError::Message(format!("route not bound: {protocol}")))?;

        let stored = self
            .db
            .get_usage_provider(&binding.provider_id)?
            .ok_or_else(|| {
                AppError::Message(format!("route config incomplete: {}", binding.provider_id))
            })?;

        if !stored.enabled {
            return Err(AppError::Message(format!(
                "route provider disabled: {}",
                stored.id
            )));
        }
        if stored.billing_kind != BillingKind::Metered {
            return Err(AppError::Message(format!(
                "route provider is not metered: {}",
                stored.id
            )));
        }
        if stored
            .route_app_type
            .as_deref()
            .is_some_and(|route_app_type| route_app_type != protocol)
        {
            return Err(AppError::Message(format!(
                "route config incomplete: {}",
                stored.id
            )));
        }

        let route_config = stored
            .route_config
            .clone()
            .filter(|config| config.as_object().is_some_and(|object| !object.is_empty()))
            .ok_or_else(|| AppError::Message(format!("route config incomplete: {}", stored.id)))?;

        let route_config = normalize_direct_route_config(route_config);

        if let (Some(legacy_app_type), Some(legacy_provider_id)) = (
            stored.legacy_app_type.as_deref(),
            stored.legacy_provider_id.as_deref(),
        ) {
            if let Some(mut provider) = self
                .db
                .get_provider_by_id(legacy_provider_id, legacy_app_type)?
            {
                // v13 is the routing SSOT. Keep the legacy envelope only for
                // adapter metadata and compatibility-log identity; otherwise
                // edits made in the dashboard would continue using stale v12
                // endpoint/credential data.
                provider.name = stored.name;
                provider.settings_config = route_config;
                validate_runtime_route(protocol, &stored.id, &provider)?;
                return Ok(BoundProvider {
                    usage_provider_id: stored.id,
                    provider,
                });
            }
        }

        let provider = Provider::with_id(stored.id.clone(), stored.name, route_config, None);
        validate_runtime_route(protocol, &stored.id, &provider)?;
        Ok(BoundProvider {
            usage_provider_id: stored.id,
            provider,
        })
    }

    /// 请求执行前获取熔断器“放行许可”
    ///
    /// - Closed：直接放行
    /// - Open：超时到达后切到 HalfOpen 并放行一次探测
    /// - HalfOpen：按限流规则放行探测
    ///
    /// 注意：调用方必须在请求结束后通过 `record_result()` 释放 HalfOpen 名额，
    /// 否则会导致该 Provider 长时间无法进入探测状态。
    #[allow(dead_code)] // Retained for v12 compatibility outside the request path.
    pub async fn allow_provider_request(&self, provider_id: &str, app_type: &str) -> AllowResult {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.allow_request().await
    }

    /// 记录供应商请求结果
    #[allow(dead_code)] // Retained for v12 compatibility outside the request path.
    pub async fn record_result(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), AppError> {
        // 1. 按应用独立获取熔断器配置
        let failure_threshold = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => app_config.circuit_failure_threshold,
            Err(_) => 5, // 默认值
        };

        // 2. 更新熔断器状态
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;

        if success {
            breaker.record_success(used_half_open_permit).await;
        } else {
            breaker.record_failure(used_half_open_permit).await;
        }

        // 3. 更新数据库健康状态（使用配置的阈值）
        self.db
            .update_provider_health_with_threshold(
                provider_id,
                app_type,
                success,
                error_msg.clone(),
                failure_threshold,
            )
            .await?;

        Ok(())
    }

    /// 重置熔断器（手动恢复）
    pub async fn reset_circuit_breaker(&self, circuit_key: &str) {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(circuit_key) {
            breaker.reset().await;
        }
    }

    /// 重置指定供应商的熔断器
    pub async fn reset_provider_breaker(&self, provider_id: &str, app_type: &str) {
        let circuit_key = format!("{app_type}:{provider_id}");
        self.reset_circuit_breaker(&circuit_key).await;
    }

    /// 仅释放 HalfOpen permit，不影响健康统计（neutral 接口）
    ///
    /// 用于整流器等场景：请求结果不应计入 Provider 健康度，
    /// 但仍需释放占用的探测名额，避免 HalfOpen 状态卡死
    #[allow(dead_code)] // Retained for v12 compatibility outside the request path.
    pub async fn release_permit_neutral(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
    ) {
        if !used_half_open_permit {
            return;
        }
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.release_half_open_permit();
    }

    /// 更新所有熔断器的配置（热更新）
    pub async fn update_all_configs(&self, config: CircuitBreakerConfig) {
        let breakers = self.circuit_breakers.read().await;
        for breaker in breakers.values() {
            breaker.update_config(config.clone()).await;
        }
    }

    /// 更新指定应用已创建熔断器的配置（热更新）
    pub async fn update_app_configs(&self, app_type: &str, config: CircuitBreakerConfig) {
        let prefix = format!("{app_type}:");
        let breakers = self.circuit_breakers.read().await;
        for (key, breaker) in breakers.iter() {
            if key.starts_with(&prefix) {
                breaker.update_config(config.clone()).await;
            }
        }
    }

    /// 获取熔断器状态
    #[allow(dead_code)]
    pub async fn get_circuit_breaker_stats(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Option<crate::proxy::circuit_breaker::CircuitBreakerStats> {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breakers = self.circuit_breakers.read().await;

        if let Some(breaker) = breakers.get(&circuit_key) {
            Some(breaker.get_stats().await)
        } else {
            None
        }
    }

    /// 获取或创建熔断器
    async fn get_or_create_circuit_breaker(&self, key: &str) -> Arc<CircuitBreaker> {
        // 先尝试读锁获取
        {
            let breakers = self.circuit_breakers.read().await;
            if let Some(breaker) = breakers.get(key) {
                return breaker.clone();
            }
        }

        // 如果不存在，获取写锁创建
        let mut breakers = self.circuit_breakers.write().await;

        // 双重检查，防止竞争条件
        if let Some(breaker) = breakers.get(key) {
            return breaker.clone();
        }

        // 从 key 中提取 app_type (格式: "app_type:provider_id")
        let app_type = key.split(':').next().unwrap_or("claude");

        // 按应用独立读取熔断器配置
        let config = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => crate::proxy::circuit_breaker::CircuitBreakerConfig {
                failure_threshold: app_config.circuit_failure_threshold,
                success_threshold: app_config.circuit_success_threshold,
                timeout_seconds: app_config.circuit_timeout_seconds as u64,
                error_rate_threshold: app_config.circuit_error_rate_threshold,
                min_requests: app_config.circuit_min_requests,
            },
            Err(_) => crate::proxy::circuit_breaker::CircuitBreakerConfig::default(),
        };

        let breaker = Arc::new(CircuitBreaker::new(config));
        breakers.insert(key.to_string(), breaker.clone());

        breaker
    }
}

fn normalize_direct_route_config(mut config: serde_json::Value) -> serde_json::Value {
    if let Some(object) = config.as_object_mut() {
        if !object.contains_key("base_url") && !object.contains_key("baseURL") {
            if let Some(base_url) = object.get("baseUrl").cloned() {
                object.insert("base_url".to_string(), base_url);
            }
        }
    }
    config
}

fn validate_runtime_route(
    protocol: &str,
    usage_provider_id: &str,
    provider: &Provider,
) -> Result<(), AppError> {
    let app_type = AppType::from_str(protocol)
        .map_err(|_| AppError::Message(format!("route config incomplete: {usage_provider_id}")))?;
    let adapter = get_adapter(&app_type);
    let base_url = adapter
        .extract_base_url(provider)
        .map_err(|_| AppError::Message(format!("route config incomplete: {usage_provider_id}")))?;
    let parsed = reqwest::Url::parse(base_url.trim())
        .map_err(|_| AppError::Message(format!("route config incomplete: {usage_provider_id}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Message(format!(
            "route config incomplete: {usage_provider_id}"
        )));
    }
    if adapter.extract_auth(provider).is_none() {
        return Err(AppError::Message(format!(
            "route config incomplete: {usage_provider_id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("CC_SWITCH_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("CC_SWITCH_TEST_HOME", dir.path());
            crate::settings::reload_settings().expect("reload settings");

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("CC_SWITCH_TEST_HOME", value),
                None => env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_provider_router_creation() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let router = ProviderRouter::new(db);

        let breaker = router.get_or_create_circuit_breaker("claude:test").await;
        assert!(breaker.allow_request().await.allowed);
    }

    fn usage_provider(
        id: &str,
        billing_kind: BillingKind,
        route_config: Option<serde_json::Value>,
    ) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: format!("Usage Provider {id}"),
            billing_kind,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("claude".to_string()),
            route_config,
            quota_config: None,
            enabled: true,
        }
    }

    #[tokio::test]
    #[serial]
    async fn bound_usage_provider_wins_over_old_current_and_failover_state() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_c =
            Provider::with_id("c".to_string(), "Provider C".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_c).unwrap();
        db.set_current_provider("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "c").unwrap();
        db.save_usage_provider(&usage_provider(
            "b",
            BillingKind::Metered,
            Some(json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://bound.example",
                    "ANTHROPIC_AUTH_TOKEN": "secret"
                }
            })),
        ))
        .unwrap();
        db.set_route_binding("claude", "b").unwrap();

        let router = ProviderRouter::new(db.clone());
        let provider = router.select_bound_provider("claude").await.unwrap();

        assert_eq!(provider.id, "b");
        assert_eq!(provider.name, "Usage Provider b");
    }

    #[tokio::test]
    #[serial]
    async fn missing_route_binding_is_rejected_locally() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let error = ProviderRouter::new(db)
            .select_bound_provider("claude")
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "route not bound: claude");
    }

    #[tokio::test]
    #[serial]
    async fn disabled_bound_provider_is_rejected_on_the_next_selection() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let input = usage_provider(
            "bound",
            BillingKind::Metered,
            Some(json!({"baseUrl": "https://bound.example", "apiKey": "secret"})),
        );
        db.save_usage_provider(&input).unwrap();
        db.set_route_binding("claude", "bound").unwrap();

        let router = ProviderRouter::new(db.clone());
        assert_eq!(
            router.select_bound_provider("claude").await.unwrap().id,
            "bound"
        );

        db.set_usage_provider_enabled("bound", false).unwrap();
        let error = router.select_bound_provider("claude").await.unwrap_err();
        assert_eq!(error.to_string(), "route provider disabled: bound");
    }

    #[tokio::test]
    #[serial]
    async fn subscription_mutation_is_rejected_on_the_next_selection() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let mut input = usage_provider(
            "bound",
            BillingKind::Metered,
            Some(json!({"baseUrl": "https://bound.example", "apiKey": "secret"})),
        );
        db.save_usage_provider(&input).unwrap();
        db.set_route_binding("claude", "bound").unwrap();

        let router = ProviderRouter::new(db.clone());
        assert!(router.select_bound_provider("claude").await.is_ok());

        input.billing_kind = BillingKind::Subscription;
        db.save_usage_provider(&input).unwrap();
        let error = router.select_bound_provider("claude").await.unwrap_err();
        assert_eq!(error.to_string(), "route provider is not metered: bound");
    }

    #[tokio::test]
    #[serial]
    async fn incomplete_direct_route_configs_are_rejected() {
        for (index, route_config) in [
            None,
            Some(json!("not-an-object")),
            Some(json!({})),
            Some(json!({"baseUrl": "https://bound.example"})),
            Some(json!({"apiKey": "secret"})),
            Some(json!({"baseUrl": "not a url", "apiKey": "secret"})),
        ]
        .into_iter()
        .enumerate()
        {
            let _home = TempHome::new();
            let db = Arc::new(Database::memory().unwrap());
            let id = format!("incomplete-{index}");
            db.save_usage_provider(&usage_provider(&id, BillingKind::Metered, route_config))
                .unwrap();
            db.set_route_binding("claude", &id).unwrap();

            let error = ProviderRouter::new(db)
                .select_bound_provider("claude")
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), format!("route config incomplete: {id}"));
        }
    }

    #[tokio::test]
    #[serial]
    async fn incomplete_v13_route_config_is_rejected_even_when_legacy_is_complete() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let legacy = Provider::with_id(
            "legacy-complete".to_string(),
            "Legacy Complete".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://legacy.example",
                    "ANTHROPIC_AUTH_TOKEN": "legacy-secret"
                }
            }),
            None,
        );
        db.save_provider("claude", &legacy).unwrap();
        db.save_usage_provider(&usage_provider(
            "claude:legacy-complete",
            BillingKind::Metered,
            Some(json!({"baseUrl": "https://stored.example"})),
        ))
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers
                 SET legacy_app_type = 'claude', legacy_provider_id = 'legacy-complete'
                 WHERE id = 'claude:legacy-complete'",
                [],
            )
            .unwrap();
        }
        db.set_route_binding("claude", "claude:legacy-complete")
            .unwrap();

        let error = ProviderRouter::new(db)
            .select_bound_provider("claude")
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "route config incomplete: claude:legacy-complete"
        );
    }

    #[tokio::test]
    #[serial]
    async fn direct_route_normalizes_public_base_url_alias_for_the_adapter() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&usage_provider(
            "direct",
            BillingKind::Metered,
            Some(json!({
                "baseUrl": "https://direct.example/v1",
                "apiKey": "secret"
            })),
        ))
        .unwrap();
        db.set_route_binding("claude", "direct").unwrap();

        let bound = ProviderRouter::new(db)
            .select_bound_route("claude")
            .await
            .unwrap();

        assert_eq!(bound.usage_provider_id, "direct");
        assert_eq!(
            bound.provider.settings_config["base_url"].as_str(),
            Some("https://direct.example/v1")
        );
    }

    #[tokio::test]
    #[serial]
    async fn legacy_route_uses_v13_config_and_preserves_runtime_identity() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let legacy_settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://legacy.example",
                "ANTHROPIC_AUTH_TOKEN": "legacy-secret"
            }
        });
        let legacy = Provider::with_id(
            "legacy-id".to_string(),
            "Legacy".to_string(),
            legacy_settings,
            None,
        );
        db.save_provider("claude", &legacy).unwrap();
        db.save_usage_provider(&usage_provider(
            "claude:legacy-id",
            BillingKind::Metered,
            Some(json!({
                "baseUrl": "https://edited.example/v1",
                "apiKey": "edited-secret"
            })),
        ))
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers
                 SET legacy_app_type='claude', legacy_provider_id='legacy-id'
                 WHERE id='claude:legacy-id'",
                [],
            )
            .unwrap();
        }
        db.set_route_binding("claude", "claude:legacy-id").unwrap();

        let bound = ProviderRouter::new(db)
            .select_bound_route("claude")
            .await
            .unwrap();

        assert_eq!(bound.usage_provider_id, "claude:legacy-id");
        assert_eq!(bound.provider.id, "legacy-id");
        assert_eq!(
            bound.provider.settings_config["base_url"].as_str(),
            Some("https://edited.example/v1")
        );
        assert_eq!(
            bound.provider.settings_config["apiKey"].as_str(),
            Some("edited-secret")
        );
        assert!(bound.provider.settings_config.get("env").is_none());
    }

    #[tokio::test]
    #[serial]
    async fn route_app_type_must_match_the_requested_protocol() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let mut input = usage_provider(
            "wrong-protocol",
            BillingKind::Metered,
            Some(json!({"baseUrl": "https://bound.example", "apiKey": "secret"})),
        );
        input.route_app_type = Some("codex".to_string());
        db.save_usage_provider(&input).unwrap();
        db.set_route_binding("claude", "wrong-protocol").unwrap();

        let error = ProviderRouter::new(db)
            .select_bound_provider("claude")
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "route config incomplete: wrong-protocol");
    }

    #[tokio::test]
    #[serial]
    async fn test_release_permit_neutral_frees_half_open_slot() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // 配置熔断器：1 次失败即熔断，0 秒超时立即进入 HalfOpen
        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        db.save_provider("claude", &provider_a).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // 启用自动故障转移
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        // 触发熔断：1 次失败
        router
            .record_result("a", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        // 第一次请求：获取 HalfOpen 探测名额
        let first = router.allow_provider_request("a", "claude").await;
        assert!(first.allowed);
        assert!(first.used_half_open_permit);

        // 第二次请求应被拒绝（名额已被占用）
        let second = router.allow_provider_request("a", "claude").await;
        assert!(!second.allowed);

        // 使用 release_permit_neutral 释放名额（不影响健康统计）
        router
            .release_permit_neutral("a", "claude", first.used_half_open_permit)
            .await;

        // 第三次请求应被允许（名额已释放）
        let third = router.allow_provider_request("a", "claude").await;
        assert!(third.allowed);
        assert!(third.used_half_open_permit);
    }
}
