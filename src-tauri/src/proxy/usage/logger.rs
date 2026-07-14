//! Usage Logger - 记录 API 请求使用情况

use super::calculator::{CostBreakdown, CostCalculator, ModelPricing};
use super::parser::TokenUsage;
use crate::credentials::CredentialExposureGuard;
use crate::database::{Database, PRICING_SOURCE_REQUEST, PRICING_SOURCE_RESPONSE};
use crate::error::AppError;
use crate::proxy::provider_router::BindingPricingOverride;
use crate::services::usage_stats::{find_model_pricing_row, is_placeholder_pricing_model};
use crate::usage::ingestion::{UsageIngestionInput, UsageIngestionOutcome, UsageIngestionService};
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvalidPricingConfigKind {
    DefaultMultiplier,
    DefaultPricingSource,
    ProviderMultiplier,
    ProviderPricingSource,
}

fn invalid_pricing_config_warning(kind: InvalidPricingConfigKind, context: &str) -> String {
    match kind {
        InvalidPricingConfigKind::DefaultMultiplier => {
            format!("[USG-003] 默认倍率配置无效 (app_type={context})")
        }
        InvalidPricingConfigKind::DefaultPricingSource => {
            format!("[USG-003] 默认计费模式配置无效 (app_type={context})")
        }
        InvalidPricingConfigKind::ProviderMultiplier => {
            format!("[USG-003] 供应商倍率配置无效 (provider_id={context})")
        }
        InvalidPricingConfigKind::ProviderPricingSource => {
            format!("[USG-003] 供应商计费模式配置无效 (provider_id={context})")
        }
    }
}

fn canonical_pricing_app_type(app_type: &str) -> &str {
    if app_type == "claude-desktop" {
        "claude"
    } else {
        app_type
    }
}

/// 请求日志
#[derive(Debug, Clone)]
pub struct RequestLog {
    pub request_id: String,
    pub provider_id: String,
    pub app_type: String,
    pub model: String,
    pub request_model: String,
    /// 写入时实际用于计价的模型名（pricing_model_source 解析后的结果）。
    /// 落库供回填使用：缺价行补价后必须按写入时的基准重算，而不是
    /// 用 model/request_model 猜——路由接管下三者可能各不相同。
    /// 错误行（未计价）为空字符串。
    pub pricing_model: String,
    pub usage: TokenUsage,
    pub cost: Option<CostBreakdown>,
    pub latency_ms: u64,
    pub first_token_ms: Option<u64>,
    pub status_code: u16,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    /// 供应商类型 (claude, claude_auth, codex, gemini, gemini_cli, openrouter)
    pub provider_type: Option<String>,
    /// 是否为流式请求
    pub is_streaming: bool,
    /// 成本倍数
    pub cost_multiplier: String,
}

#[allow(clippy::too_many_arguments)]
fn request_log_values_contain_credential(
    credential_guard: &CredentialExposureGuard,
    log: &RequestLog,
    input_cost: &str,
    output_cost: &str,
    cache_read_cost: &str,
    cache_creation_cost: &str,
    total_cost: &str,
    created_at: i64,
) -> bool {
    [
        log.request_id.as_str(),
        log.provider_id.as_str(),
        log.app_type.as_str(),
        log.model.as_str(),
        log.request_model.as_str(),
        log.pricing_model.as_str(),
        input_cost,
        output_cost,
        cache_read_cost,
        cache_creation_cost,
        total_cost,
        log.cost_multiplier.as_str(),
    ]
    .into_iter()
    .any(|value| credential_guard.contains(value))
        || [
            log.error_message.as_deref(),
            log.session_id.as_deref(),
            log.provider_type.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| credential_guard.contains(value))
        || [
            log.usage.input_tokens.to_string(),
            log.usage.output_tokens.to_string(),
            log.usage.cache_read_tokens.to_string(),
            log.usage.cache_creation_tokens.to_string(),
            log.latency_ms.to_string(),
            log.first_token_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            log.status_code.to_string(),
            i64::from(log.is_streaming).to_string(),
            created_at.to_string(),
        ]
        .into_iter()
        .filter(|value| !value.is_empty())
        .any(|value| credential_guard.contains(&value))
}

/// 使用量记录器
pub struct UsageLogger<'a> {
    db: &'a Database,
}

impl<'a> UsageLogger<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub(crate) fn ingest_with_credential_guard(
        &self,
        input: &UsageIngestionInput,
        credential_guard: &CredentialExposureGuard,
    ) -> Result<UsageIngestionOutcome, AppError> {
        UsageIngestionService::new(self.db).ingest_with_credential_guard(input, credential_guard)
    }

    /// 记录成功的请求
    pub fn log_request(&self, log: &RequestLog) -> Result<(), AppError> {
        self.log_request_internal(log, None)
    }

    fn log_request_internal(
        &self,
        log: &RequestLog,
        credential_guard: Option<&CredentialExposureGuard>,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(self.db.conn);

        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) =
            if let Some(cost) = &log.cost {
                (
                    cost.input_cost.to_string(),
                    cost.output_cost.to_string(),
                    cost.cache_read_cost.to_string(),
                    cost.cache_creation_cost.to_string(),
                    cost.total_cost.to_string(),
                )
            } else {
                (
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                )
            };

        let created_at = chrono::Utc::now().timestamp();

        if credential_guard.is_some_and(|credential_guard| {
            request_log_values_contain_credential(
                credential_guard,
                log,
                &input_cost,
                &output_cost,
                &cache_read_cost,
                &cache_creation_cost,
                &total_cost,
                created_at,
            )
        }) {
            log::warn!("Request log omitted because persistence would repeat protected credential material");
            return Ok(());
        }

        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model, pricing_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                latency_ms, first_token_ms, status_code, error_message, session_id,
                provider_type, is_streaming, cost_multiplier, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
            ON CONFLICT(request_id) DO NOTHING",
            rusqlite::params![
                log.request_id,
                log.provider_id,
                log.app_type,
                log.model,
                log.request_model,
                log.pricing_model,
                log.usage.input_tokens,
                log.usage.output_tokens,
                log.usage.cache_read_tokens,
                log.usage.cache_creation_tokens,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                log.latency_ms as i64,
                log.first_token_ms.map(|v| v as i64),
                log.status_code as i64,
                log.error_message,
                log.session_id,
                log.provider_type,
                log.is_streaming as i64,
                log.cost_multiplier,
                created_at,
            ],
        )
        .map_err(|e| AppError::Database(format!("记录请求日志失败: {e}")))?;

        // 通知前端使用统计有更新（200ms 防抖合并，不阻塞写入路径）
        crate::usage_events::notify_log_recorded();

        Ok(())
    }

    /// 记录失败的请求
    ///
    /// 用于记录无法从上游获取 usage 信息的失败请求
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn log_error(
        &self,
        request_id: String,
        provider_id: String,
        app_type: String,
        model: String,
        status_code: u16,
        error_message: String,
        latency_ms: u64,
    ) -> Result<(), AppError> {
        let request_model = model.clone();
        let log = RequestLog {
            request_id,
            provider_id,
            app_type,
            model,
            request_model,
            // 错误行未经过计价，留空（回填的 has_usage 闸门也不会碰全 0 行）
            pricing_model: String::new(),
            usage: TokenUsage::default(),
            cost: None,
            latency_ms,
            first_token_ms: None,
            status_code,
            error_message: Some(error_message),
            session_id: None,
            provider_type: None,
            is_streaming: false,
            cost_multiplier: "1.0".to_string(),
        };

        self.log_request(&log)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn log_error_with_context_guarded(
        &self,
        request_id: String,
        provider_id: String,
        app_type: String,
        model: String,
        status_code: u16,
        error_message: String,
        latency_ms: u64,
        is_streaming: bool,
        session_id: Option<String>,
        provider_type: Option<String>,
        credential_guard: &CredentialExposureGuard,
    ) -> Result<(), AppError> {
        let request_model = model.clone();
        let log = RequestLog {
            request_id,
            provider_id,
            app_type,
            model,
            request_model,
            pricing_model: String::new(),
            usage: TokenUsage::default(),
            cost: None,
            latency_ms,
            first_token_ms: None,
            status_code,
            error_message: Some(error_message),
            session_id,
            provider_type,
            is_streaming,
            cost_multiplier: "1.0".to_string(),
        };

        self.log_request_internal(&log, Some(credential_guard))
    }

    /// 获取模型定价
    #[allow(dead_code)] // Retained for v12 compatibility logging.
    pub fn get_model_pricing(&self, model_id: &str) -> Result<Option<ModelPricing>, AppError> {
        let conn = crate::database::lock_conn!(self.db.conn);
        let row = find_model_pricing_row(&conn, model_id)?;
        match row {
            Some((input, output, cache_read, cache_creation)) => {
                ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation)
                    .map(Some)
                    .map_err(|e| AppError::Database(format!("解析定价数据失败: {e}")))
            }
            None => Ok(None),
        }
    }

    /// 获取有效的倍率与计费模式来源（供应商优先，未配置则回退全局默认）
    pub async fn resolve_pricing_config(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> (Decimal, String) {
        let default_app_type = canonical_pricing_app_type(app_type);
        let provider = self
            .db
            .get_provider_by_id(provider_id, default_app_type)
            .ok()
            .flatten();
        let (provider_multiplier, provider_pricing_source) = provider
            .as_ref()
            .and_then(|provider| provider.meta.as_ref())
            .map(|meta| {
                (
                    meta.cost_multiplier.as_deref(),
                    meta.pricing_model_source.as_deref(),
                )
            })
            .unwrap_or((None, None));
        self.resolve_pricing_values(
            provider_multiplier,
            provider_pricing_source,
            Some(provider_id),
            app_type,
        )
        .await
    }

    pub(crate) async fn resolve_binding_pricing_config(
        &self,
        pricing_override: &BindingPricingOverride,
        app_type: &str,
    ) -> (Decimal, String) {
        self.resolve_pricing_values(
            pricing_override.cost_multiplier.as_deref(),
            pricing_override.pricing_model_source.as_deref(),
            None,
            app_type,
        )
        .await
    }

    async fn resolve_pricing_values(
        &self,
        provider_multiplier: Option<&str>,
        provider_pricing_source: Option<&str>,
        provider_context: Option<&str>,
        app_type: &str,
    ) -> (Decimal, String) {
        // Claude Desktop 网关没有独立的全局计费配置（proxy_config 的 CHECK 仅
        // 允许 claude/codex/gemini，前端也只暴露三项），计费默认与供应商级
        // meta 覆盖都从 canonical claude 分区解析。
        let default_app_type = canonical_pricing_app_type(app_type);
        let default_multiplier_raw =
            match self.db.get_default_cost_multiplier(default_app_type).await {
                Ok(value) => value,
                Err(e) => {
                    log::warn!("[USG-003] 获取默认倍率失败 (app_type={app_type}): {e}");
                    "1".to_string()
                }
            };
        let default_multiplier = match Decimal::from_str(&default_multiplier_raw) {
            Ok(value) => value,
            Err(_) => {
                log::warn!(
                    "{}",
                    invalid_pricing_config_warning(
                        InvalidPricingConfigKind::DefaultMultiplier,
                        app_type,
                    )
                );
                Decimal::from(1)
            }
        };

        let default_pricing_source_raw =
            match self.db.get_pricing_model_source(default_app_type).await {
                Ok(value) => value,
                Err(e) => {
                    log::warn!("[USG-003] 获取默认计费模式失败 (app_type={app_type}): {e}");
                    PRICING_SOURCE_RESPONSE.to_string()
                }
            };
        let default_pricing_source = if default_pricing_source_raw == PRICING_SOURCE_RESPONSE
            || default_pricing_source_raw == PRICING_SOURCE_REQUEST
        {
            default_pricing_source_raw
        } else {
            log::warn!(
                "{}",
                invalid_pricing_config_warning(
                    InvalidPricingConfigKind::DefaultPricingSource,
                    app_type,
                )
            );
            PRICING_SOURCE_RESPONSE.to_string()
        };

        let cost_multiplier = match provider_multiplier {
            Some(value) => match Decimal::from_str(value) {
                Ok(parsed) => parsed,
                Err(_) => {
                    log::warn!(
                        "{}",
                        invalid_pricing_config_warning(
                            InvalidPricingConfigKind::ProviderMultiplier,
                            provider_context.unwrap_or("binding-migration"),
                        )
                    );
                    default_multiplier
                }
            },
            None => default_multiplier,
        };

        let pricing_model_source = match provider_pricing_source {
            Some(value) if value == PRICING_SOURCE_RESPONSE || value == PRICING_SOURCE_REQUEST => {
                value.to_string()
            }
            Some(_) => {
                log::warn!(
                    "{}",
                    invalid_pricing_config_warning(
                        InvalidPricingConfigKind::ProviderPricingSource,
                        provider_context.unwrap_or("binding-migration"),
                    )
                );
                default_pricing_source.clone()
            }
            None => default_pricing_source.clone(),
        };

        (cost_multiplier, pricing_model_source)
    }

    /// 计算并记录请求
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Retained for compatibility callers outside the v13 request path.
    pub fn log_with_calculation(
        &self,
        request_id: String,
        provider_id: String,
        app_type: String,
        model: String,
        request_model: String,
        pricing_model: String,
        usage: TokenUsage,
        cost_multiplier: Decimal,
        latency_ms: u64,
        first_token_ms: Option<u64>,
        status_code: u16,
        session_id: Option<String>,
        provider_type: Option<String>,
        is_streaming: bool,
    ) -> Result<(), AppError> {
        let pricing = self.get_model_pricing(&pricing_model)?;

        let has_usage = usage.input_tokens > 0
            || usage.output_tokens > 0
            || usage.cache_read_tokens > 0
            || usage.cache_creation_tokens > 0;

        if pricing.is_none() && has_usage && !is_placeholder_pricing_model(&pricing_model) {
            log::warn!("[USG-002] 模型定价未找到，成本将记录为 0: {pricing_model}");
        }

        let cost = CostCalculator::try_calculate_for_app(
            &app_type,
            &usage,
            pricing.as_ref(),
            cost_multiplier,
        );

        let log = RequestLog {
            request_id,
            provider_id,
            app_type,
            model,
            request_model,
            pricing_model,
            usage,
            cost,
            latency_ms,
            first_token_ms,
            status_code,
            error_message: None,
            session_id,
            provider_type,
            is_streaming,
            cost_multiplier: cost_multiplier.to_string(),
        };

        self.log_request(&log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claude_desktop_pricing_uses_canonical_claude_provider_override() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        db.set_default_cost_multiplier("claude", "1.5").await?;
        db.set_pricing_model_source("claude", PRICING_SOURCE_RESPONSE)
            .await?;

        let provider_meta = serde_json::json!({
            "costMultiplier": "2",
            "pricingModelSource": PRICING_SOURCE_REQUEST,
        })
        .to_string();
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "claude-desktop-provider",
                    "claude",
                    "Claude Desktop Provider",
                    "{}",
                    provider_meta,
                ],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        }

        let logger = UsageLogger::new(&db);
        let (multiplier, pricing_source) = logger
            .resolve_pricing_config("claude-desktop-provider", "claude-desktop")
            .await;

        assert_eq!(multiplier, Decimal::from(2));
        assert_eq!(pricing_source, PRICING_SOURCE_REQUEST);
        Ok(())
    }

    #[test]
    fn invalid_pricing_config_warnings_expose_only_fixed_context() {
        let cases = [
            (
                InvalidPricingConfigKind::DefaultMultiplier,
                "claude",
                "[USG-003] 默认倍率配置无效 (app_type=claude)",
            ),
            (
                InvalidPricingConfigKind::DefaultPricingSource,
                "codex",
                "[USG-003] 默认计费模式配置无效 (app_type=codex)",
            ),
            (
                InvalidPricingConfigKind::ProviderMultiplier,
                "provider-1",
                "[USG-003] 供应商倍率配置无效 (provider_id=provider-1)",
            ),
            (
                InvalidPricingConfigKind::ProviderPricingSource,
                "provider-2",
                "[USG-003] 供应商计费模式配置无效 (provider_id=provider-2)",
            ),
        ];

        for (kind, context, expected) in cases {
            let warning = invalid_pricing_config_warning(kind, context);
            assert_eq!(warning, expected);
            assert!(!warning.contains("SECRET_INVALID_CONFIG_SENTINEL"));
            assert!(!warning.contains("invalid decimal parse details"));
        }
    }

    #[test]
    fn test_log_request() -> Result<(), AppError> {
        let db = Database::memory()?;

        // 插入测试定价
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('test-model', 'Test Model', '3.0', '15.0')",
                [],
            )
            .unwrap();
        }

        let logger = UsageLogger::new(&db);

        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        logger.log_with_calculation(
            "req-123".to_string(),
            "provider-1".to_string(),
            "claude".to_string(),
            "test-model".to_string(),
            "req-model".to_string(),
            "test-model".to_string(),
            usage,
            Decimal::from(1),
            100,
            None,
            200,
            None,
            Some("claude".to_string()),
            false,
        )?;

        // 验证记录已插入
        let conn = crate::database::lock_conn!(db.conn);
        let (count, request_model): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), request_model FROM proxy_request_logs WHERE request_id = 'req-123'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(request_model, "req-model");
        Ok(())
    }

    #[test]
    fn test_log_error() -> Result<(), AppError> {
        let db = Database::memory()?;
        let logger = UsageLogger::new(&db);

        logger.log_error(
            "req-error".to_string(),
            "provider-1".to_string(),
            "claude".to_string(),
            "unknown-model".to_string(),
            500,
            "Internal Server Error".to_string(),
            50,
        )?;

        // 验证错误记录已插入
        let conn = crate::database::lock_conn!(db.conn);
        let (status, error): (i64, Option<String>) = conn
            .query_row(
                "SELECT status_code, error_message FROM proxy_request_logs WHERE request_id = 'req-error'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, 500);
        assert_eq!(error, Some("Internal Server Error".to_string()));
        Ok(())
    }

    #[test]
    fn guarded_error_log_drops_numeric_status_that_repeats_binding_key() -> Result<(), AppError> {
        let db = Database::memory()?;
        let logger = UsageLogger::new(&db);

        logger.log_error_with_context_guarded(
            "guarded-error".to_string(),
            "provider-1".to_string(),
            "claude".to_string(),
            "unknown-model".to_string(),
            401,
            "request failed".to_string(),
            50,
            false,
            None,
            None,
            &CredentialExposureGuard::from_secret(b"401"),
        )?;

        let count: i64 = crate::database::lock_conn!(db.conn).query_row(
            "SELECT COUNT(*) FROM proxy_request_logs",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);
        Ok(())
    }
}
