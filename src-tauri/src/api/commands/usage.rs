//! 使用统计相关命令

use crate::app_state::AppState;
use crate::error::AppError;
use crate::policy::{community_license_status, LicenseStatus};
use crate::store::Database;
use crate::usage::trend::{clamp_legacy_usage_query, require_usage_split};
use crate::usage::usage_stats::*;
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use std::str::FromStr;
use tauri::State;

/// Refresh the machine-maintained official model price catalogue.
#[tauri::command]
pub async fn refresh_official_pricing(
    state: State<'_, AppState>,
) -> Result<crate::providers::shared::official_pricing::RefreshOutcome, AppError> {
    crate::providers::shared::official_pricing::refresh_official_pricing(&state.db).await
}

/// Return the timestamp of the last successful official-price refresh.
#[tauri::command]
pub fn get_official_pricing_last_refresh_at(
    state: State<'_, AppState>,
) -> Result<Option<i64>, AppError> {
    crate::providers::shared::official_pricing::last_refresh_at(&state.db)
}

#[tauri::command]
pub fn get_official_pricing_last_imported_count(
    state: State<'_, AppState>,
) -> Result<Option<u64>, AppError> {
    crate::providers::shared::official_pricing::last_imported_count(&state.db)
}

/// 获取使用量汇总
#[tauri::command]
pub fn get_usage_summary(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<UsageSummary, AppError> {
    legacy_usage_summary_at(
        &state.db,
        start_date,
        end_date,
        app_type.as_deref(),
        provider_name.as_deref(),
        model.as_deref(),
        &community_license_status(),
        Local::now(),
    )
}

#[allow(clippy::too_many_arguments)]
fn legacy_usage_summary_at(
    db: &Database,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<&str>,
    provider_name: Option<&str>,
    model: Option<&str>,
    license: &LicenseStatus,
    now: DateTime<Local>,
) -> Result<UsageSummary, AppError> {
    let range = clamp_legacy_usage_query(start_date, end_date, true, license, now)?;
    let mut summary =
        db.get_usage_summary(range.start, range.end, app_type, provider_name, model)?;
    summary.clamped_from = range.clamped_from;
    Ok(summary)
}

/// 获取按 app_type 拆分的使用量汇总
#[tauri::command]
pub fn get_usage_summary_by_app(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<UsageSummaryByApp>, AppError> {
    // app_type is the legacy agent/application breakdown, not an agent filter.
    require_usage_split(&community_license_status())?;
    state.db.get_usage_summary_by_app(
        start_date,
        end_date,
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// Daily trends (ADR v7.19.1). Entitlement is server-owned, never a request field.
#[tauri::command]
pub fn query_usage_trend(
    state: State<'_, AppState>,
    tz: String,
    from: String,
    to: String,
    granularity: String,
    group_by: Option<String>,
    filters: Option<serde_json::Value>,
) -> Result<crate::usage::trend::UsageTrendResponse, AppError> {
    crate::usage::trend::query_usage_trend_at(
        &state.db,
        serde_json::json!({
            "tz": tz, "from": from, "to": to, "granularity": granularity,
            "groupBy": group_by, "filters": filters,
        }),
        &crate::policy::community_license_status(),
        chrono::Utc::now(),
    )
}

/// Deprecated: legacy trend API. Use `query_usage_trend` for the v7.19 contract.
/// Keeps `Vec<DailyStats>` and decimal-string costs; Free silently clamps dates.
#[tauri::command]
pub fn get_usage_trends(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<DailyStats>, AppError> {
    legacy_usage_trends_at(
        &state.db,
        start_date,
        end_date,
        app_type.as_deref(),
        provider_name.as_deref(),
        model.as_deref(),
        &community_license_status(),
        Local::now(),
    )
}

#[allow(clippy::too_many_arguments)]
fn legacy_usage_trends_at(
    db: &Database,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<&str>,
    provider_name: Option<&str>,
    model: Option<&str>,
    license: &LicenseStatus,
    now: DateTime<Local>,
) -> Result<Vec<DailyStats>, AppError> {
    let range = clamp_legacy_usage_query(start_date, end_date, true, license, now)?;
    // The store's equal/reversed-range fallback expands to 24h. Do not let an
    // empty Free intersection expand backwards outside the permitted window.
    if range.empty
        || (crate::usage::trend::usage_query_is_limited(license) && range.start == range.end)
    {
        return Ok(Vec::new());
    }
    db.get_daily_trends(range.start, range.end, app_type, provider_name, model)
}

/// 获取 Provider 统计
#[tauri::command]
pub fn get_provider_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ProviderStats>, AppError> {
    require_usage_split(&community_license_status())?;
    state.db.get_provider_stats(
        start_date,
        end_date,
        app_type.as_deref(),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// 获取模型统计
#[tauri::command]
pub fn get_model_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ModelStats>, AppError> {
    require_usage_split(&community_license_status())?;
    state.db.get_model_stats(
        start_date,
        end_date,
        app_type.as_deref(),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// 获取请求日志列表
#[tauri::command]
pub fn get_request_logs(
    state: State<'_, AppState>,
    filters: LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs, AppError> {
    state.db.get_request_logs(&filters, page, page_size)
}

/// 获取单个请求详情
#[tauri::command]
pub fn get_request_detail(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<RequestLogDetail>, AppError> {
    state.db.get_request_detail(&request_id)
}

/// 获取模型定价列表
#[tauri::command]
pub fn get_model_pricing(state: State<'_, AppState>) -> Result<Vec<ModelPricingInfo>, AppError> {
    log::info!("获取模型定价列表");
    state.db.ensure_model_pricing_seeded()?;

    let db = state.db.clone();
    let conn = crate::store::lock_conn!(db.conn);

    // 检查表是否存在
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_pricing'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .unwrap_or(false);

    if !table_exists {
        log::error!("model_pricing 表不存在,可能需要重启应用以触发数据库迁移");
        return Ok(Vec::new());
    }

    // Preserve NULL: missing cache rates are estimated at input price, not free.
    let mut stmt = conn.prepare(
        "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         ORDER BY display_name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(ModelPricingInfo {
            model_id: row.get(0)?,
            display_name: row.get(1)?,
            input_cost_per_million: row.get(2)?,
            output_cost_per_million: row.get(3)?,
            cache_read_cost_per_million: row.get(4)?,
            cache_creation_cost_per_million: row.get(5)?,
        })
    })?;

    let mut pricing = Vec::new();
    for row in rows {
        pricing.push(row?);
    }

    log::info!("成功获取 {} 条模型定价数据", pricing.len());
    Ok(pricing)
}

/// 更新模型定价
#[tauri::command]
pub fn update_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_creation_cost: String,
) -> Result<(), AppError> {
    let db = state.db.clone();
    let model_id = model_id.trim().to_string();
    let display_name = display_name.trim().to_string();
    if model_id.is_empty() {
        return Err(AppError::localized(
            "usage.modelIdRequired",
            "模型 ID 不能为空",
            "Model ID is required",
        ));
    }
    if display_name.is_empty() {
        return Err(AppError::localized(
            "usage.displayNameRequired",
            "显示名称不能为空",
            "Display name is required",
        ));
    }

    for (label, value) in [
        ("input_cost", &input_cost),
        ("output_cost", &output_cost),
        ("cache_read_cost", &cache_read_cost),
        ("cache_creation_cost", &cache_creation_cost),
    ] {
        let parsed = Decimal::from_str(value.trim()).map_err(|e| {
            AppError::localized(
                "usage.invalidPrice",
                format!("{label} 价格无效: {value} - {e}"),
                format!("{label} price is invalid: {value} - {e}"),
            )
        })?;
        if parsed < Decimal::ZERO {
            return Err(AppError::localized(
                "usage.invalidPrice",
                format!("{label} 价格必须为非负数: {value}"),
                format!("{label} price must be non-negative: {value}"),
            ));
        }
    }

    {
        let conn = crate::store::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                model_id,
                display_name,
                input_cost.trim(),
                output_cost.trim(),
                cache_read_cost.trim(),
                cache_creation_cost.trim()
            ],
        )
        .map_err(|e| AppError::Database(format!("更新模型定价失败: {e}")))?;
    }

    if let Err(e) = db.backfill_missing_usage_costs_for_model(&model_id) {
        log::warn!("模型定价更新后回填历史用量成本失败 (model_id={model_id}): {e}");
    }

    Ok(())
}

/// 获取某个 Provider 账号的自定义模型定价（用户实付价）
#[tauri::command]
pub fn get_provider_model_pricing(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Vec<crate::model::ProviderModelPricingView>, AppError> {
    state.db.list_provider_model_pricing(&provider_id)
}

/// 保存某个 Provider 账号对某个模型的自定义定价
///
/// 只影响之后新采集的用量；已入库的事件保持采集当时的价格不变。
#[tauri::command]
pub fn update_provider_model_pricing(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
    display_name: String,
    price: crate::model::ModelPriceInput,
) -> Result<(), AppError> {
    state
        .db
        .upsert_provider_model_pricing(&provider_id, &model_id, &display_name, &price)
}

/// 删除某个 Provider 账号对某个模型的自定义定价，之后回落到上游成本或官方价
#[tauri::command]
pub fn delete_provider_model_pricing(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
) -> Result<(), AppError> {
    state
        .db
        .delete_provider_model_pricing(&provider_id, &model_id)
}

/// 检查 Provider 使用限额
#[tauri::command]
pub fn check_provider_limits(
    state: State<'_, AppState>,
    provider_id: String,
    app_type: String,
) -> Result<crate::usage::usage_stats::ProviderLimitStatus, AppError> {
    state.db.check_provider_limits(&provider_id, &app_type)
}

/// 删除模型定价
#[tauri::command]
pub fn delete_model_pricing(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    let db = state.db.clone();
    let conn = crate::store::lock_conn!(db.conn);

    conn.execute(
        "DELETE FROM model_pricing WHERE model_id = ?1",
        rusqlite::params![model_id],
    )
    .map_err(|e| AppError::Database(format!("删除模型定价失败: {e}")))?;

    log::info!("已删除模型定价: {model_id}");
    Ok(())
}

/// 手动触发会话日志同步
#[tauri::command]
pub fn sync_session_usage(
    state: State<'_, AppState>,
) -> Result<crate::ingest::SessionSyncResult, AppError> {
    // 同步 Claude 会话日志
    let mut result = crate::ingest::claude::sync_claude_session_logs(&state.db)?;

    // 同步 Codex 使用数据
    match crate::ingest::codex::sync_codex_usage(&state.db) {
        Ok(codex_result) => {
            result.imported += codex_result.imported;
            result.skipped += codex_result.skipped;
            result.files_scanned += codex_result.files_scanned;
            result.files_pruned += codex_result.files_pruned;
            result.errors.extend(codex_result.errors);
        }
        Err(e) => {
            result.errors.push(format!("Codex 同步失败: {e}"));
        }
    }

    // 同步 Gemini 使用数据
    match crate::ingest::gemini::sync_gemini_usage(&state.db) {
        Ok(gemini_result) => {
            result.imported += gemini_result.imported;
            result.skipped += gemini_result.skipped;
            result.files_scanned += gemini_result.files_scanned;
            result.files_pruned += gemini_result.files_pruned;
            result.errors.extend(gemini_result.errors);
        }
        Err(e) => {
            result.errors.push(format!("Gemini 同步失败: {e}"));
        }
    }

    // 同步 OpenCode 使用数据
    match crate::ingest::opencode::sync_opencode_usage(&state.db) {
        Ok(opencode_result) => {
            result.imported += opencode_result.imported;
            result.skipped += opencode_result.skipped;
            result.files_scanned += opencode_result.files_scanned;
            result.files_pruned += opencode_result.files_pruned;
            result.errors.extend(opencode_result.errors);
        }
        Err(e) => {
            result.errors.push(format!("OpenCode 同步失败: {e}"));
        }
    }

    Ok(result)
}

/// 获取数据来源分布
#[tauri::command]
pub fn get_usage_data_sources(
    state: State<'_, AppState>,
) -> Result<Vec<crate::ingest::DataSourceSummary>, AppError> {
    crate::ingest::get_data_source_breakdown(&state.db)
}

/// 模型定价信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingInfo {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: Option<String>,
    pub cache_creation_cost_per_million: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{LicenseBuild, LicenseTier};
    use chrono::TimeZone;

    #[test]
    fn legacy_summary_free_bounds_metadata_filters_and_pro_passthrough() {
        let db = Database::memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let lower = Local
            .with_ymd_and_hms(2026, 9, 27, 0, 0, 0)
            .unwrap()
            .timestamp();
        let upper = Local
            .with_ymd_and_hms(2026, 10, 27, 0, 0, 0)
            .unwrap()
            .timestamp();
        for (id, at, tokens) in [
            ("old", lower - 1, 100),
            ("first", lower, 7),
            ("last", upper - 1, 13),
            ("future", upper, 1000),
        ] {
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, created_at, input_tokens,
                  total_cost_usd, latency_ms, status_code)
                 VALUES (?1, 'p', 'claude', 'm', ?2, ?3, '0.25', 0, 200)",
                    rusqlite::params![id, at, tokens],
                )
                .unwrap();
        }
        let free = community_license_status();
        for start in [None, Some(lower - 1), Some(lower), Some(lower + 1)] {
            let summary = legacy_usage_summary_at(
                &db,
                start,
                None,
                Some("claude"),
                None,
                Some("m"),
                &free,
                now,
            )
            .unwrap();
            let json = serde_json::to_value(&summary).unwrap();
            assert_eq!(
                summary.real_total_tokens,
                if start == Some(lower + 1) { 13 } else { 20 }
            );
            assert_eq!(
                json.get("clampedFrom").and_then(|v| v.as_str()),
                if start.is_none() || start == Some(lower - 1) {
                    Some("2026-09-27")
                } else {
                    None
                }
            );
        }
        // An upper-only clamp must not invent lower-bound metadata.
        let end_only = legacy_usage_summary_at(
            &db,
            Some(lower),
            Some(upper + 1),
            None,
            None,
            None,
            &free,
            now,
        )
        .unwrap();
        assert_eq!(end_only.total_requests, 2);
        assert!(end_only.clamped_from.is_none());
        // Request-log filters are exempt, including historical model filters.
        let logs = db.get_request_logs(&LogFilters::default(), 0, 20).unwrap();
        assert_eq!(logs.total, 4);
        let logs = db
            .get_request_logs(
                &LogFilters {
                    model: Some("m".into()),
                    end_date: Some(lower - 1),
                    ..Default::default()
                },
                0,
                20,
            )
            .unwrap();
        assert_eq!(logs.total, 1);
        let mut pro = free.clone();
        pro.build = LicenseBuild::Official;
        pro.tier = LicenseTier::Pro;
        let summary =
            legacy_usage_summary_at(&db, None, None, None, None, None, &pro, now).unwrap();
        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.real_total_tokens, 1120);
        assert!(!serde_json::to_value(summary)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("clampedFrom"));
    }

    #[test]
    fn legacy_trends_keep_vec_costs_hourly_and_empty_intersections() {
        let db = Database::memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let lower = Local
            .with_ymd_and_hms(2026, 9, 27, 0, 0, 0)
            .unwrap()
            .timestamp();
        for (id, at, tokens) in [("before", lower - 1, 100), ("inside", lower + 1, 7)] {
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, created_at, input_tokens,
                  total_cost_usd, latency_ms, status_code)
                 VALUES (?1, 'p', 'claude', 'm', ?2, ?3, '0.25', 0, 200)",
                    rusqlite::params![id, at, tokens],
                )
                .unwrap();
        }
        let free = community_license_status();
        let daily = legacy_usage_trends_at(&db, None, None, None, None, None, &free, now).unwrap();
        assert_eq!(daily.len(), 30);
        assert!(daily[0].date.starts_with("2026-09-27"));
        assert!(serde_json::to_value(&daily).unwrap().is_array());
        assert_eq!(daily.iter().map(|day| day.total_tokens).sum::<u64>(), 7);
        assert!(daily
            .iter()
            .all(|day| day.total_cost.parse::<f64>().is_ok()));
        let hourly = legacy_usage_trends_at(
            &db,
            Some(now.timestamp() - 7200),
            Some(now.timestamp()),
            None,
            None,
            None,
            &free,
            now,
        )
        .unwrap();
        assert_eq!(hourly.len(), 2);
        for (start, end) in [
            (lower - 86400, lower - 1),
            (lower, lower),
            (now.timestamp() + 172800, now.timestamp() + 259200),
        ] {
            assert!(legacy_usage_trends_at(
                &db,
                Some(start),
                Some(end),
                None,
                None,
                None,
                &free,
                now
            )
            .unwrap()
            .is_empty());
        }
        let mut pro = free;
        pro.build = LicenseBuild::Official;
        pro.tier = LicenseTier::ProP3;
        let actual = legacy_usage_trends_at(
            &db,
            Some(lower - 86400),
            Some(lower - 1),
            None,
            None,
            None,
            &pro,
            now,
        )
        .unwrap();
        let old = db
            .get_daily_trends(Some(lower - 86400), Some(lower - 1), None, None, None)
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(old).unwrap()
        );
    }
}
