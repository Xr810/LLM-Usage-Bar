//! 供应商路由器模块
//!
//! 负责选择和管理代理目标供应商，实现智能故障转移

use crate::app_config::AppType;
use crate::database::{
    resolve_direct_api_format, resolve_direct_credential_placement, Database,
    DirectCredentialPlacement, PRICING_SOURCE_REQUEST, PRICING_SOURCE_RESPONSE,
};
use crate::error::AppError;
use crate::provider::{ClaudeDesktopModelRoute, Provider, ProviderMeta};
use crate::proxy::binding_auth::is_credential_query_name;
use crate::proxy::circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};
use crate::proxy::providers::get_adapter;
#[cfg(test)]
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
#[cfg(test)]
pub struct BoundProvider {
    /// Global v13 identity used by usage_events and quota/source ownership.
    pub usage_provider_id: String,
    /// Transitional Provider used by the existing protocol adapters.
    pub provider: Provider,
}

/// Header location used to inject the protected binding credential upstream.
///
/// This carries no credential material and is derived only from the protocol
/// and non-secret route metadata before the legacy configuration is scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamCredentialPlacement {
    AuthorizationBearer,
    XApiKey,
    XGoogApiKey,
}

pub(crate) struct BindingRouteProjection {
    pub(crate) runtime_provider: Provider,
    pub(crate) credential_placement: UpstreamCredentialPlacement,
    pub(crate) pricing_override: BindingPricingOverride,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BindingPricingOverride {
    pub(crate) cost_multiplier: Option<String>,
    pub(crate) pricing_model_source: Option<String>,
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_credential_field(key: &str) -> bool {
    let key = normalized_key(key);
    matches!(
        key.as_str(),
        "auth"
            | "authentication"
            | "authorization"
            | "apikey"
            | "token"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "bearertoken"
            | "experimentalbearertoken"
            | "secret"
            | "secretaccesskey"
            | "sessiontoken"
            | "password"
            | "credential"
            | "credentials"
            | "privatekey"
            | "signingkey"
            | "accesskeyid"
            | "cookie"
            | "xapikey"
            | "xgoogapikey"
    ) || key.ends_with("apikey")
        || key.ends_with("authorization")
        || key.ends_with("token")
        || key.ends_with("secret")
        || key.ends_with("password")
        || key.ends_with("credential")
        || key.ends_with("privatekey")
        || key.ends_with("accesskeyid")
        || key.ends_with("authtoken")
        || key.ends_with("accesstoken")
        || key.ends_with("refreshtoken")
        || key.ends_with("bearertoken")
        || key.ends_with("secretaccesskey")
        || key.ends_with("credentials")
        || key.ends_with("subscriptionkey")
        || key.ends_with("auth")
        || key == "headers"
        || key.ends_with("headers")
        || key.contains("bearertoken")
        || key.contains("credential")
        || key.contains("clientsecret")
        || key.contains("secretaccesskey")
}

fn sanitized_absolute_url(raw: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(raw).ok()?;
    url.host_str()?;
    if !url.username().is_empty() {
        url.set_username("").ok()?;
    }
    if url.password().is_some() {
        url.set_password(None).ok()?;
    }
    let retained_query = url
        .query_pairs()
        .filter(|(key, _)| !is_credential_query_name(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !retained_query.is_empty() {
        url.query_pairs_mut().extend_pairs(retained_query);
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn sanitized_relative_url(raw: &str) -> Option<String> {
    let (without_fragment, removed_fragment) = raw
        .split_once('#')
        .map(|(value, _)| (value, true))
        .unwrap_or((raw, false));
    let (path, query) = without_fragment.split_once('?')?;
    let mut probe = reqwest::Url::parse("https://binding-route.invalid/").ok()?;
    probe.set_query(Some(query));
    let pairs = probe
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let retained = pairs
        .iter()
        .filter(|(key, _)| !is_credential_query_name(key))
        .cloned()
        .collect::<Vec<_>>();
    if retained.len() == pairs.len() && !removed_fragment {
        return None;
    }
    probe.set_query(None);
    if !retained.is_empty() {
        probe.query_pairs_mut().extend_pairs(retained);
    }
    let retained_query = probe.query().unwrap_or_default();
    Some(if retained_query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{retained_query}")
    })
}

fn sanitized_url(raw: &str) -> Option<String> {
    sanitized_absolute_url(raw).or_else(|| sanitized_relative_url(raw))
}

fn sanitize_toml_value(value: &mut toml::Value) {
    match value {
        toml::Value::Table(table) => {
            let removed = table
                .keys()
                .filter(|key| is_credential_field(key))
                .cloned()
                .collect::<Vec<_>>();
            for key in removed {
                table.remove(&key);
            }
            for (_, value) in table.iter_mut() {
                sanitize_toml_value(value);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                sanitize_toml_value(value);
            }
        }
        toml::Value::String(value) => {
            if let Some(sanitized) = sanitized_url(value) {
                *value = sanitized;
            }
        }
        toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Boolean(_)
        | toml::Value::Datetime(_) => {}
    }
}

fn sanitized_toml_config(raw: &str) -> String {
    let Ok(mut config) = toml::from_str::<toml::Value>(raw) else {
        // An opaque config string cannot be proven credential-free.
        return String::new();
    };
    sanitize_toml_value(&mut config);
    toml::to_string(&config).unwrap_or_default()
}

fn sanitize_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            let removed = object
                .keys()
                .filter(|key| is_credential_field(key))
                .cloned()
                .collect::<Vec<_>>();
            for key in removed {
                object.remove(&key);
            }
            for (key, value) in object {
                if normalized_key(key) == "config" {
                    if let Some(config) = value.as_str() {
                        *value = serde_json::Value::String(sanitized_toml_config(config));
                        continue;
                    }
                }
                sanitize_json_value(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                sanitize_json_value(value);
            }
        }
        serde_json::Value::String(value) => {
            if let Some(sanitized) = sanitized_url(value) {
                *value = sanitized;
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn matching_top_level_value<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<Option<&'a serde_json::Value>, AppError> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let mut matches = object
        .iter()
        .filter(|(candidate, _)| normalized_key(candidate) == key)
        .map(|(_, value)| value);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(AppError::Message("invalid_binding".to_string()));
    }
    Ok(first)
}

fn canonical_api_format(value: &serde_json::Value) -> Result<String, AppError> {
    let Some(value) = value.as_str() else {
        return Err(AppError::Message("invalid_binding".to_string()));
    };
    match normalized_key(value).as_str() {
        "anthropic" => Ok("anthropic".to_string()),
        "openaichat" => Ok("openai_chat".to_string()),
        "openairesponses" => Ok("openai_responses".to_string()),
        "gemininative" => Ok("gemini_native".to_string()),
        _ => Err(AppError::Message("invalid_binding".to_string())),
    }
}

fn top_level_bool(value: &serde_json::Value, key: &str) -> Result<Option<bool>, AppError> {
    matching_top_level_value(value, key)?
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| AppError::Message("invalid_binding".to_string()))
        })
        .transpose()
}

fn take_top_level_desktop_model_routes(
    route_config: &mut serde_json::Value,
) -> Result<Option<HashMap<String, ClaudeDesktopModelRoute>>, AppError> {
    let Some(object) = route_config.as_object_mut() else {
        return Ok(None);
    };
    let matching_keys = object
        .keys()
        .filter(|key| normalized_key(key) == "claudedesktopmodelroutes")
        .cloned()
        .collect::<Vec<_>>();
    if matching_keys.len() > 1 {
        return Err(AppError::Message("invalid_binding".to_string()));
    }
    let Some(key) = matching_keys.into_iter().next() else {
        return Ok(None);
    };
    let routes = object.remove(&key).unwrap_or(serde_json::Value::Null);
    serde_json::from_value(routes)
        .map(Some)
        .map_err(|_| AppError::Message("invalid_binding".to_string()))
}

/// Merge only the three route-affecting fields that v12 stored in Provider
/// metadata. The v13 route config is authoritative per field; the migration
/// copy in quota_config is preferred over the still-present legacy row only
/// when that field is absent from route_config.
fn merge_safe_migrated_route_metadata(
    route_config: &mut serde_json::Value,
    migration_meta: Option<&serde_json::Value>,
    legacy_meta: Option<&ProviderMeta>,
) -> Result<(), AppError> {
    if route_config.as_object().is_none() {
        return Err(AppError::Message("invalid_binding".to_string()));
    }

    if matching_top_level_value(route_config, "apiformat")?.is_none() {
        let fallback = match migration_meta
            .map(|meta| matching_top_level_value(meta, "apiformat"))
            .transpose()?
            .flatten()
        {
            Some(value) => Some(canonical_api_format(value)?),
            None => legacy_meta
                .and_then(|meta| meta.api_format.as_ref())
                .map(|value| canonical_api_format(&serde_json::Value::String(value.clone())))
                .transpose()?,
        };
        if let Some(value) = fallback {
            route_config
                .as_object_mut()
                .expect("route config object checked above")
                .insert("apiFormat".to_string(), serde_json::Value::String(value));
        }
    }

    if matching_top_level_value(route_config, "isfullurl")?.is_none() {
        let fallback = match migration_meta
            .map(|meta| matching_top_level_value(meta, "isfullurl"))
            .transpose()?
            .flatten()
        {
            Some(value) => Some(
                value
                    .as_bool()
                    .ok_or_else(|| AppError::Message("invalid_binding".to_string()))?,
            ),
            None => legacy_meta.and_then(|meta| meta.is_full_url),
        };
        if let Some(value) = fallback {
            route_config
                .as_object_mut()
                .expect("route config object checked above")
                .insert("isFullUrl".to_string(), serde_json::Value::Bool(value));
        }
    }

    if matching_top_level_value(route_config, "claudedesktopmodelroutes")?.is_none() {
        let fallback = match migration_meta
            .map(|meta| matching_top_level_value(meta, "claudedesktopmodelroutes"))
            .transpose()?
            .flatten()
        {
            Some(value) => Some(
                serde_json::from_value::<HashMap<String, ClaudeDesktopModelRoute>>(value.clone())
                    .map_err(|_| AppError::Message("invalid_binding".to_string()))?,
            ),
            None => legacy_meta
                .filter(|meta| !meta.claude_desktop_model_routes.is_empty())
                .map(|meta| meta.claude_desktop_model_routes.clone()),
        };
        if let Some(routes) = fallback {
            let routes = serde_json::to_value(routes)
                .map_err(|_| AppError::Message("invalid_binding".to_string()))?;
            route_config
                .as_object_mut()
                .expect("route config object checked above")
                .insert("claudeDesktopModelRoutes".to_string(), routes);
        }
    }

    Ok(())
}

fn parse_safe_cost_multiplier(value: &serde_json::Value) -> Option<String> {
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        _ => return None,
    };
    rust_decimal::Decimal::from_str(&raw)
        .ok()
        .map(|value| value.to_string())
}

fn parse_safe_pricing_model_source(value: &serde_json::Value) -> Option<String> {
    match value.as_str()? {
        PRICING_SOURCE_REQUEST => Some(PRICING_SOURCE_REQUEST.to_string()),
        PRICING_SOURCE_RESPONSE => Some(PRICING_SOURCE_RESPONSE.to_string()),
        _ => None,
    }
}

fn resolve_binding_pricing_override(
    migration_meta: Option<&serde_json::Value>,
    legacy_meta: Option<&ProviderMeta>,
) -> Result<BindingPricingOverride, AppError> {
    let migrated_multiplier = migration_meta
        .map(|meta| matching_top_level_value(meta, "costmultiplier"))
        .transpose()?
        .flatten();
    let cost_multiplier = match migrated_multiplier {
        Some(value) => parse_safe_cost_multiplier(value),
        None => legacy_meta
            .and_then(|meta| meta.cost_multiplier.as_deref())
            .and_then(|value| {
                parse_safe_cost_multiplier(&serde_json::Value::String(value.to_string()))
            }),
    };

    let migrated_pricing_source = migration_meta
        .map(|meta| matching_top_level_value(meta, "pricingmodelsource"))
        .transpose()?
        .flatten();
    let pricing_model_source = match migrated_pricing_source {
        Some(value) => parse_safe_pricing_model_source(value),
        None => legacy_meta
            .and_then(|meta| meta.pricing_model_source.as_deref())
            .and_then(|value| {
                parse_safe_pricing_model_source(&serde_json::Value::String(value.to_string()))
            }),
    };

    Ok(BindingPricingOverride {
        cost_multiplier,
        pricing_model_source,
    })
}

pub(crate) fn build_binding_route_projection(
    protocol: &str,
    usage_provider_id: &str,
    provider_name: String,
    mut route_config: serde_json::Value,
    migration_meta: Option<serde_json::Value>,
    legacy_migration_linked: bool,
    legacy_provider: Option<Provider>,
) -> Result<BindingRouteProjection, AppError> {
    AppType::from_str(protocol).map_err(|_| AppError::Message("invalid_binding".to_string()))?;
    if route_config
        .as_object()
        .is_none_or(|object| object.is_empty())
    {
        return Err(AppError::Message("invalid_binding".to_string()));
    }
    let legacy_migration_linked = legacy_migration_linked || legacy_provider.is_some();
    let pricing_override = if legacy_migration_linked {
        resolve_binding_pricing_override(
            migration_meta.as_ref(),
            legacy_provider
                .as_ref()
                .and_then(|provider| provider.meta.as_ref()),
        )?
    } else {
        BindingPricingOverride::default()
    };
    merge_safe_migrated_route_metadata(
        &mut route_config,
        if legacy_migration_linked {
            migration_meta.as_ref()
        } else {
            None
        },
        if legacy_migration_linked {
            legacy_provider
                .as_ref()
                .and_then(|provider| provider.meta.as_ref())
        } else {
            None
        },
    )?;
    let credential_placement = match resolve_direct_credential_placement(protocol, &route_config)
        .ok_or_else(|| AppError::Message("invalid_binding".to_string()))?
    {
        DirectCredentialPlacement::AuthorizationBearer => {
            UpstreamCredentialPlacement::AuthorizationBearer
        }
        DirectCredentialPlacement::XApiKey => UpstreamCredentialPlacement::XApiKey,
        DirectCredentialPlacement::XGoogApiKey => UpstreamCredentialPlacement::XGoogApiKey,
    };

    let route_api_format = resolve_direct_api_format(&route_config).map(str::to_string);
    let route_is_full_url = top_level_bool(&route_config, "isfullurl")?;
    let route_desktop_model_routes =
        take_top_level_desktop_model_routes(&mut route_config)?.unwrap_or_default();
    let mut route_config = normalize_direct_route_config(route_config);
    sanitize_json_value(&mut route_config);

    let mut provider = legacy_provider.unwrap_or_else(|| {
        Provider::with_id(
            usage_provider_id.to_string(),
            provider_name.clone(),
            serde_json::Value::Null,
            None,
        )
    });
    provider.name = provider_name;
    provider.settings_config = route_config;
    // The runtime projection is a route, not a display Provider. Drop every
    // free-text/display field from the legacy envelope so a historical key in
    // notes/icon metadata cannot survive the credential scrubber.
    provider.website_url = None;
    provider.category = None;
    provider.created_at = None;
    provider.sort_index = None;
    provider.notes = None;
    provider.icon = None;
    provider.icon_color = None;
    provider.in_failover_queue = false;
    // Runtime routing uses an allowlist, not a credential blacklist. In
    // particular, legacy request overrides, custom user agents, prompt cache
    // keys, auth bindings, scripts, and account metadata are outbound-mutating
    // or free-text surfaces and must never enter a binding-owned request.
    provider.meta = Some(ProviderMeta {
        claude_desktop_model_routes: route_desktop_model_routes,
        api_format: route_api_format,
        is_full_url: route_is_full_url,
        ..Default::default()
    });

    validate_credential_free_runtime_route(protocol, usage_provider_id, &provider)?;
    Ok(BindingRouteProjection {
        runtime_provider: provider,
        credential_placement,
        pricing_override,
    })
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
    #[cfg(test)]
    pub async fn select_bound_provider(&self, protocol: &str) -> Result<Provider, AppError> {
        self.select_bound_route(protocol)
            .await
            .map(|bound| bound.provider)
    }

    #[cfg(test)]
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

fn validate_credential_free_runtime_route(
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
    Ok(())
}

#[cfg(test)]
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
            let original_test_home = env::var("LLM_USAGE_BAR_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("LLM_USAGE_BAR_TEST_HOME", dir.path());
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
                Some(value) => env::set_var("LLM_USAGE_BAR_TEST_HOME", value),
                None => env::remove_var("LLM_USAGE_BAR_TEST_HOME"),
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

    #[test]
    fn binding_route_projection_removes_old_credentials_without_mutating_legacy_provider() {
        let sentinel = "OLD_ROUTE_SECRET_SENTINEL";
        let legacy_settings = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://legacy-user:legacy-pass@legacy.example/v1?key=legacy-key&region=eu",
                "ANTHROPIC_AUTH_TOKEN": sentinel
            },
            "auth": {"OPENAI_API_KEY": sentinel},
            "nested": {
                "headers": {
                    "Authorization": format!("Bearer {sentinel}"),
                    "x-api-key": sentinel
                }
            }
        });
        let mut legacy = Provider::with_id(
            "legacy-id".to_string(),
            "Legacy name".to_string(),
            legacy_settings,
            Some("https://site-user:site-pass@provider.example?token=old&lang=en".to_string()),
        );
        legacy.category = Some("relay".to_string());
        legacy.notes = Some(format!("legacy note containing {sentinel}"));
        legacy.icon = Some(format!("legacy-icon-{sentinel}"));
        legacy.icon_color = Some(format!("#{sentinel}"));
        legacy.meta = Some(
            serde_json::from_value(json!({
                "apiFormat": "openai_chat",
                "costMultiplier": "1.25",
                "apiKeyField": "ANTHROPIC_AUTH_TOKEN",
                "testConfig": {"apiKey": sentinel, "model": "safe-model"},
                "customUserAgent": format!("legacy-agent-{sentinel}"),
                "promptCacheKey": format!("legacy-cache-{sentinel}"),
                "localProxyRequestOverrides": {
                    "headers": {"X-Relay-Auth": sentinel},
                    "body": {"legacyRelayValue": sentinel}
                }
            }))
            .unwrap(),
        );
        let original_legacy = serde_json::to_value(&legacy).unwrap();

        let projection = build_binding_route_projection(
            "claude",
            "usage-provider-id",
            "Frozen provider name".to_string(),
            json!({
                "apiFormat": "openai_chat",
                "baseUrl": "https://route-user:route-pass@upstream.example/v1?api_key=old&region=apac",
                "apiKey": sentinel,
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": sentinel,
                    "PUBLIC_HINT": "preserved"
                },
                "auth": {"OPENAI_API_KEY": sentinel},
                "model": "safe-model",
                "callback": format!("/v1/callback?api_key={sentinel}&region=apac"),
                "headers": {"X-Relay-Auth": sentinel},
                "nested": [{
                    "access_token": sentinel,
                    "client_secret": sentinel,
                    "privateKey": sentinel,
                    "legacySecret": sentinel,
                    "safe": "kept"
                }]
            }),
            None,
            false,
            Some(legacy.clone()),
        )
        .unwrap();

        assert_eq!(
            projection.credential_placement,
            UpstreamCredentialPlacement::AuthorizationBearer
        );
        assert_eq!(projection.runtime_provider.id, "legacy-id");
        assert_eq!(projection.runtime_provider.name, "Frozen provider name");
        assert!(projection.runtime_provider.category.is_none());
        assert!(projection.runtime_provider.notes.is_none());
        assert!(projection.runtime_provider.icon.is_none());
        assert!(projection.runtime_provider.icon_color.is_none());
        assert_eq!(
            projection.runtime_provider.settings_config["base_url"].as_str(),
            Some("https://upstream.example/v1?region=apac")
        );
        assert_eq!(
            projection.runtime_provider.settings_config["env"]["PUBLIC_HINT"].as_str(),
            Some("preserved")
        );
        assert_eq!(
            projection.runtime_provider.settings_config["nested"][0]["safe"].as_str(),
            Some("kept")
        );
        assert_eq!(
            projection
                .runtime_provider
                .meta
                .as_ref()
                .and_then(|meta| meta.api_format.as_deref()),
            Some("openai_chat")
        );
        assert!(projection
            .runtime_provider
            .meta
            .as_ref()
            .and_then(|meta| meta.cost_multiplier.as_deref())
            .is_none());
        assert!(projection.runtime_provider.website_url.is_none());

        let serialized_projection = serde_json::to_string(&projection.runtime_provider).unwrap();
        for forbidden in [
            sentinel,
            "route-user",
            "route-pass",
            "legacy-user",
            "legacy-pass",
            "site-user",
            "site-pass",
            "legacy-key",
        ] {
            assert!(
                !serialized_projection.contains(forbidden),
                "runtime projection leaked {forbidden}"
            );
        }
        assert_eq!(serde_json::to_value(&legacy).unwrap(), original_legacy);
    }

    #[test]
    fn binding_route_projection_uses_only_authoritative_desktop_routes_and_drops_legacy_meta() {
        let sentinel = "LEGACY_META_OUTBOUND_SENTINEL";
        let mut legacy = Provider::with_id(
            "legacy-id".to_string(),
            "Legacy name".to_string(),
            json!({"env": {"ANTHROPIC_BASE_URL": "https://legacy.example"}}),
            None,
        );
        legacy.meta = Some(
            serde_json::from_value(json!({
                "claudeDesktopModelRoutes": {
                    "legacy-route": {"model": sentinel}
                },
                "isFullUrl": true,
                "costMultiplier": sentinel,
                "pricingModelSource": sentinel,
                "codexFastMode": true,
                "codexChatReasoning": {
                    "thinkingParam": sentinel,
                    "outputFormat": sentinel
                }
            }))
            .unwrap(),
        );

        let projection = build_binding_route_projection(
            "claude-desktop",
            "usage-provider-id",
            "Frozen provider name".to_string(),
            json!({
                "apiFormat": "anthropic",
                "isFullUrl": false,
                "baseUrl": "https://upstream.example",
                "authMode": "x-api-key",
                "claudeDesktopModelRoutes": {
                    "safe-route": {
                        "model": "authoritative-model",
                        "labelOverride": "Safe route",
                        "supports1m": true
                    }
                }
            }),
            Some(json!({
                "apiFormat": "openai_responses",
                "isFullUrl": true,
                "claudeDesktopModelRoutes": {
                    "migrated-route": {"model": sentinel}
                },
                "customUserAgent": sentinel
            })),
            false,
            Some(legacy),
        )
        .unwrap();

        assert!(projection
            .runtime_provider
            .settings_config
            .as_object()
            .unwrap()
            .keys()
            .all(|key| normalized_key(key) != "claudedesktopmodelroutes"));

        let meta = projection.runtime_provider.meta.as_ref().unwrap();
        assert_eq!(meta.api_format.as_deref(), Some("anthropic"));
        assert_eq!(meta.is_full_url, Some(false));
        assert_eq!(
            meta.claude_desktop_model_routes
                .get("safe-route")
                .map(|route| route.model.as_str()),
            Some("authoritative-model")
        );
        assert!(!meta
            .claude_desktop_model_routes
            .contains_key("legacy-route"));
        assert!(!meta
            .claude_desktop_model_routes
            .contains_key("migrated-route"));
        assert!(meta.cost_multiplier.is_none());
        assert!(meta.pricing_model_source.is_none());
        assert!(meta.codex_fast_mode.is_none());
        assert!(meta.codex_chat_reasoning.is_none());
        assert!(!serde_json::to_string(&projection.runtime_provider)
            .unwrap()
            .contains(sentinel));
    }

    #[test]
    fn binding_route_projection_recovers_safe_metadata_from_a_real_v12_migration() {
        let db = Database::memory().unwrap();
        let unsafe_sentinel = "MIGRATED_UNSAFE_META_SENTINEL";
        let mut legacy = Provider::with_id(
            "migrated-route".to_string(),
            "Migrated route".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://migrated.example/v1/messages",
                    "ANTHROPIC_AUTH_TOKEN": "legacy-secret"
                }
            }),
            None,
        );
        legacy.meta = Some(
            serde_json::from_value(json!({
                "apiFormat": "gemini_native",
                "isFullUrl": true,
                "claudeDesktopModelRoutes": {
                    "migrated-route": {
                        "model": "migrated-model",
                        "labelOverride": "Migrated route"
                    }
                },
                "customUserAgent": unsafe_sentinel,
                "promptCacheKey": unsafe_sentinel,
                "localProxyRequestOverrides": {
                    "headers": {"X-Unsafe-Legacy": unsafe_sentinel}
                }
            }))
            .unwrap(),
        );
        db.save_provider("claude", &legacy).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            crate::usage::migration::migrate_v12_to_v13(&conn).unwrap();
        }

        let stored = db
            .get_usage_provider("claude:migrated-route")
            .unwrap()
            .expect("migration must create the v13 provider row");
        assert!(stored
            .quota_config
            .as_ref()
            .is_some_and(|meta| meta.to_string().contains(unsafe_sentinel)));
        let legacy_migration_linked =
            stored.legacy_app_type.is_some() && stored.legacy_provider_id.is_some();
        assert!(legacy_migration_linked);
        let migrated_legacy = db
            .get_provider_by_id("migrated-route", "claude")
            .unwrap()
            .expect("migration keeps the legacy provider row");

        let projection = build_binding_route_projection(
            "claude",
            &stored.id,
            stored.name,
            stored.route_config.unwrap(),
            stored.quota_config,
            legacy_migration_linked,
            Some(migrated_legacy),
        )
        .unwrap();
        let meta = projection.runtime_provider.meta.as_ref().unwrap();

        assert_eq!(
            projection.credential_placement,
            UpstreamCredentialPlacement::XGoogApiKey
        );
        assert_eq!(meta.api_format.as_deref(), Some("gemini_native"));
        assert_eq!(meta.is_full_url, Some(true));
        assert_eq!(
            meta.claude_desktop_model_routes
                .get("migrated-route")
                .map(|route| route.model.as_str()),
            Some("migrated-model")
        );
        assert!(!serde_json::to_string(&projection.runtime_provider)
            .unwrap()
            .contains(unsafe_sentinel));
    }

    #[test]
    fn binding_route_projection_uses_safe_legacy_meta_when_migration_copy_is_unavailable() {
        let mut legacy = Provider::with_id(
            "legacy-fallback".to_string(),
            "Legacy fallback".to_string(),
            json!({"baseUrl": "https://legacy-fallback.example", "authMode": "bearer"}),
            None,
        );
        legacy.meta = Some(
            serde_json::from_value(json!({
                "apiFormat": "openai_responses",
                "isFullUrl": true,
                "claudeDesktopModelRoutes": {
                    "legacy-safe": {"model": "legacy-safe-model"}
                }
            }))
            .unwrap(),
        );

        let projection = build_binding_route_projection(
            "claude",
            "legacy-fallback",
            "Legacy fallback".to_string(),
            json!({"baseUrl": "https://authoritative.example", "authMode": "bearer"}),
            None,
            false,
            Some(legacy),
        )
        .unwrap();
        let meta = projection.runtime_provider.meta.as_ref().unwrap();

        assert_eq!(meta.api_format.as_deref(), Some("openai_responses"));
        assert_eq!(meta.is_full_url, Some(true));
        assert_eq!(
            meta.claude_desktop_model_routes
                .get("legacy-safe")
                .map(|route| route.model.as_str()),
            Some("legacy-safe-model")
        );
    }

    #[test]
    fn binding_route_projection_ignores_quota_metadata_without_a_legacy_migration_link() {
        let projection = build_binding_route_projection(
            "claude",
            "ordinary-v13-provider",
            "Ordinary provider".to_string(),
            json!({
                "baseUrl": "https://ordinary.example",
                "authMode": "x-api-key"
            }),
            Some(json!({
                "apiFormat": "gemini_native",
                "isFullUrl": true,
                "claudeDesktopModelRoutes": {
                    "quota-route": {"model": "must-not-become-a-route"}
                }
            })),
            false,
            None,
        )
        .unwrap();
        let meta = projection.runtime_provider.meta.as_ref().unwrap();

        assert_eq!(
            projection.credential_placement,
            UpstreamCredentialPlacement::XApiKey
        );
        assert!(meta.api_format.is_none());
        assert!(meta.is_full_url.is_none());
        assert!(meta.claude_desktop_model_routes.is_empty());
    }

    #[test]
    fn binding_route_projection_uses_migration_copy_when_only_the_legacy_link_survives() {
        let projection = build_binding_route_projection(
            "claude",
            "orphaned-legacy-link",
            "Migrated provider".to_string(),
            json!({"baseUrl": "https://migrated.example"}),
            Some(json!({"apiFormat": "gemini_native", "isFullUrl": true})),
            true,
            None,
        )
        .unwrap();
        let meta = projection.runtime_provider.meta.as_ref().unwrap();

        assert_eq!(
            projection.credential_placement,
            UpstreamCredentialPlacement::XGoogApiKey
        );
        assert_eq!(meta.api_format.as_deref(), Some("gemini_native"));
        assert_eq!(meta.is_full_url, Some(true));
    }

    #[test]
    fn binding_route_projection_derives_header_placement_without_reading_secret_values() {
        let cases = [
            (
                "claude",
                json!({
                    "baseUrl": "https://upstream.example",
                    "env": {"ANTHROPIC_AUTH_TOKEN": ""}
                }),
                UpstreamCredentialPlacement::AuthorizationBearer,
            ),
            (
                "claude",
                json!({
                    "baseUrl": "https://upstream.example",
                    "env": {"ANTHROPIC_API_KEY": ""}
                }),
                UpstreamCredentialPlacement::XApiKey,
            ),
            (
                "codex",
                json!({"baseUrl": "https://upstream.example", "auth": {}}),
                UpstreamCredentialPlacement::AuthorizationBearer,
            ),
            (
                "gemini",
                json!({"baseUrl": "https://upstream.example", "apiKey": ""}),
                UpstreamCredentialPlacement::XGoogApiKey,
            ),
            (
                "claude",
                json!({"baseUrl": "https://upstream.example", "authMode": "bearer"}),
                UpstreamCredentialPlacement::AuthorizationBearer,
            ),
            (
                "codex",
                json!({"baseUrl": "https://upstream.example", "authMode": "x-api-key"}),
                UpstreamCredentialPlacement::XApiKey,
            ),
            (
                "gemini",
                json!({"baseUrl": "https://upstream.example", "authentication": {"method": "bearer"}}),
                UpstreamCredentialPlacement::AuthorizationBearer,
            ),
        ];

        for (protocol, route_config, expected) in cases {
            let projection = build_binding_route_projection(
                protocol,
                "usage-provider-id",
                "Provider".to_string(),
                route_config,
                None,
                false,
                None,
            )
            .unwrap();
            assert_eq!(projection.credential_placement, expected);
            assert!(serde_json::to_string(&projection.runtime_provider)
                .unwrap()
                .find("OLD_ROUTE_SECRET_SENTINEL")
                .is_none());
        }
    }

    #[test]
    fn claude_gemini_native_route_uses_google_api_key_header() {
        for protocol in ["claude", "claude-desktop"] {
            let mut legacy = Provider::with_id(
                "legacy-gemini".to_string(),
                "Gemini Native".to_string(),
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://generativelanguage.googleapis.com",
                        "ANTHROPIC_API_KEY": "OLD_ROUTE_SECRET_SENTINEL"
                    }
                }),
                None,
            );
            legacy.meta = Some(
                serde_json::from_value(json!({
                    "apiFormat": "gemini_native",
                    "apiKeyField": "ANTHROPIC_API_KEY"
                }))
                .unwrap(),
            );

            let projection = build_binding_route_projection(
                protocol,
                "gemini-native-provider",
                "Gemini Native".to_string(),
                json!({
                    "apiFormat": "GEMINI_NATIVE",
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://generativelanguage.googleapis.com",
                        "ANTHROPIC_API_KEY": "OLD_ROUTE_SECRET_SENTINEL"
                    }
                }),
                None,
                false,
                Some(legacy),
            )
            .unwrap();

            assert_eq!(
                projection.credential_placement,
                UpstreamCredentialPlacement::XGoogApiKey,
                "{protocol}"
            );
            assert_eq!(
                projection
                    .runtime_provider
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.api_format.as_deref()),
                Some("gemini_native"),
                "{protocol}"
            );
            assert!(!serde_json::to_string(&projection.runtime_provider)
                .unwrap()
                .contains("OLD_ROUTE_SECRET_SENTINEL"));
        }
    }

    #[test]
    fn claude_route_with_ambiguous_key_markers_fails_closed() {
        let result = build_binding_route_projection(
            "claude",
            "ambiguous-provider",
            "Ambiguous".to_string(),
            json!({
                "baseUrl": "https://api.example",
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "",
                    "ANTHROPIC_API_KEY": "OLD_ROUTE_SECRET_SENTINEL"
                }
            }),
            None,
            false,
            None,
        );
        let error = match result {
            Ok(_) => panic!("ambiguous credential markers must fail closed"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "invalid_binding");
        assert!(!error.to_string().contains("OLD_ROUTE_SECRET_SENTINEL"));
    }

    #[test]
    fn codex_binding_route_projection_sanitizes_toml_auth_and_url_credentials() {
        let projection = build_binding_route_projection(
            "codex",
            "codex-provider",
            "Codex Provider".to_string(),
            json!({
                "config": r#"
model = "safe-model"
model_provider = "custom"
experimental_bearer_token = "OLD_ROUTE_SECRET_SENTINEL"

[model_providers.custom]
base_url = "https://toml-user:toml-pass@codex.example/v1?access_token=old&region=us"
experimental_bearer_token = "OLD_ROUTE_SECRET_SENTINEL"
wire_api = "responses"
"#,
                "auth": {"OPENAI_API_KEY": "OLD_ROUTE_SECRET_SENTINEL"}
            }),
            None,
            false,
            None,
        )
        .unwrap();

        assert_eq!(
            projection.credential_placement,
            UpstreamCredentialPlacement::AuthorizationBearer
        );
        let sanitized_toml = projection.runtime_provider.settings_config["config"]
            .as_str()
            .unwrap();
        let parsed: toml::Value = toml::from_str(sanitized_toml).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("safe-model"));
        assert_eq!(
            parsed["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://codex.example/v1?region=us")
        );
        assert_eq!(
            parsed["model_providers"]["custom"]["wire_api"].as_str(),
            Some("responses")
        );
        let serialized = serde_json::to_string(&projection.runtime_provider).unwrap();
        for forbidden in [
            "OLD_ROUTE_SECRET_SENTINEL",
            "experimental_bearer_token",
            "toml-user",
            "toml-pass",
            "access_token",
        ] {
            assert!(!serialized.contains(forbidden));
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
