//! Hourly usage rollups, dirty-hour rebuilds, and retention (BE-15).
//!
//! The first backfill reads `proxy_request_logs` only. It does not read
//! `usage_events`. ADR §6.3's accepted migration text backfills the hourly
//! table from `proxy_request_logs`, and this batch must not delete
//! `usage_events`. Those two tables can describe the same request, so summing
//! both would double-count. Because the detail table is the only source, the
//! community hourly default stays 400 days: this backfill cannot see the older
//! history that may still sit in `usage_events`. A later private build passes
//! 730 through [`RetentionPolicy::hourly_days`]. Free's 52-week cap is
//! [`HEATMAP_MAX_WEEKS`], applied when a query asks for a window, not by
//! deleting hourly rows.
//!
//! Costs are stored as micro-USD integers. Each raw row is converted with
//! `rust_decimal` and then summed, so the hourly total matches the raw total
//! to 0 micro-USD. Historical rows with a NULL billing identity group as `""`.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::NaiveDate;
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection, Row};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::error::AppError;
#[cfg(test)]
use crate::store::lock_conn;
use crate::store::Database;

pub const DETAIL_RETENTION_DAYS_DEFAULT: i64 = 90;
pub const DETAIL_RETENTION_DAYS_MIN: i64 = 30;
pub const DETAIL_RETENTION_DAYS_MAX: i64 = 365;
/// Community default. Prune does not read this itself; callers pass it inside
/// [`RetentionPolicy`]. A private Pro build can pass 730 instead.
pub const HOURLY_RETENTION_DAYS_DEFAULT: i64 = 400;
pub const PRUNE_BATCH_LIMIT: i64 = 5_000;
pub const DETAIL_RETENTION_SETTING_KEY: &str = "proxy_log_retention_days";
/// Heatmap window for every tier. v7.6 closed the Free / Pro split.
/// BE-16 applies this when a query asks for a window. Prune does not.
#[allow(dead_code)]
pub const HEATMAP_MAX_WEEKS: i64 = 52;

/// Windows passed into prune. Detail days follow the 30–365 setting. Hourly
/// days are not a schema constant: the caller supplies them on every prune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub detail_days: i64,
    pub hourly_days: i64,
}

/// Build-provided policy used by both startup and scheduled maintenance.
/// This is a requested retention floor, not the final pruning window. The store
/// persists and applies max(retention_high_water_days, hourly_days), including
/// after a downgrade or reopening with the community provider. Providers must
/// not use the current tier alone to decide what persisted history to delete.
pub trait RetentionPolicyProvider: Send + Sync {
    fn retention_policy(&self, detail_days: i64) -> RetentionPolicy;
}

impl<T: RetentionPolicyProvider + ?Sized> RetentionPolicyProvider for std::sync::Arc<T> {
    fn retention_policy(&self, detail_days: i64) -> RetentionPolicy {
        (**self).retention_policy(detail_days)
    }
}

pub struct CommunityRetentionPolicy;

impl RetentionPolicyProvider for CommunityRetentionPolicy {
    fn retention_policy(&self, detail_days: i64) -> RetentionPolicy {
        RetentionPolicy::community(detail_days)
    }
}

impl RetentionPolicy {
    pub const fn community(detail_days: i64) -> Self {
        Self {
            detail_days,
            hourly_days: HOURLY_RETENTION_DAYS_DEFAULT,
        }
    }
}

const END_REASON_BLOCKED: &str = "blocked";
const END_REASON_CLIENT_ABORT: &str = "client_abort";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyLogDelta {
    pub created_at: i64,
    pub app_type: String,
    pub provider_id: String,
    pub model: String,
    pub billing_identity: String,
    pub data_source: String,
    pub status_code: i64,
    pub end_reason: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub total_cost_usd: String,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct LocalDayBucket {
    pub date: NaiveDate,
    pub approximate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HourKey {
    hour_utc: i64,
    app_type: String,
    provider_id: String,
    model: String,
    billing_identity: String,
    data_source: String,
}

#[derive(Debug, Clone, Default)]
struct HourTotals {
    request_count: i64,
    success_count: i64,
    blocked_count: i64,
    abort_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    cost_micro_usd: i64,
    latency_ms_sum: i64,
}

pub fn hour_bucket_utc(created_at: i64) -> i64 {
    created_at.div_euclid(3600) * 3600
}

pub fn usd_text_to_micro(text: &str) -> i64 {
    let Ok(value) = Decimal::from_str(text.trim()) else {
        return 0;
    };
    let micro = value * Decimal::from(1_000_000);
    micro
        .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
        .to_i64()
        .unwrap_or(0)
}

/// Group a UTC hour bucket into a local calendar day.
///
/// Whole-hour offsets are exact. Offsets that are not a multiple of one hour
/// (for example UTC+5:30) are marked `approximate` because the stored bucket
/// cannot split events that fall on different local days inside the same UTC hour.
#[allow(dead_code)]
pub fn local_day_for_hour(hour_utc: i64, offset_east_seconds: i32) -> Option<LocalDayBucket> {
    let approximate = offset_east_seconds.rem_euclid(3_600) != 0;
    let local_epoch = hour_utc.checked_add(i64::from(offset_east_seconds))?;
    let day_index = local_epoch.div_euclid(86_400);
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    let date = epoch.checked_add_signed(chrono::Duration::days(day_index))?;
    Some(LocalDayBucket { date, approximate })
}

#[allow(dead_code)]
pub fn heatmap_max_weeks_for_tier(_tier: &str) -> i64 {
    HEATMAP_MAX_WEEKS
}

pub(crate) fn hourly_retention_cutoff(now_unix: i64, hourly_days: i64) -> Result<i64, AppError> {
    if hourly_days < 0 {
        return Err(AppError::invalid_argument(
            "Hourly retention days must be zero or greater.".to_string(),
            "hourlyRetentionDays",
        ));
    }
    let raw = now_unix.saturating_sub(hourly_days.saturating_mul(86_400));
    Ok(hour_bucket_utc(raw))
}

pub(crate) fn prune_hourly_rollups(
    conn: &Connection,
    now_unix: i64,
    hourly_days: i64,
) -> Result<u64, AppError> {
    prune_hourly_rollups_before(conn, hourly_retention_cutoff(now_unix, hourly_days)?)
}

pub(crate) fn rebuild_usage_hourly_rollups(conn: &Connection) -> Result<(), AppError> {
    conn.execute("DELETE FROM usage_hourly_rollups", [])
        .map_err(|error| AppError::Database(format!("清空小时聚合失败: {error}")))?;
    let totals = aggregate_logs(conn, "", params![])?;
    insert_totals(conn, totals)
}

pub(crate) fn upsert_hourly_delta(
    conn: &Connection,
    delta: &HourlyLogDelta,
) -> Result<(), AppError> {
    let mut totals = HashMap::new();
    contribute(&mut totals, delta)?;
    insert_totals(conn, totals)
}

pub(crate) fn mark_rollup_hour_dirty(conn: &Connection, created_at: i64) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO rollup_dirty_hours (hour_utc) VALUES (?1)
         ON CONFLICT(hour_utc) DO NOTHING",
        [hour_bucket_utc(created_at)],
    )
    .map_err(|error| AppError::Database(format!("标记脏小时失败: {error}")))?;
    Ok(())
}

pub(crate) fn backfill_dirty_hours(conn: &Connection) -> Result<u64, AppError> {
    let mut statement = conn
        .prepare("SELECT hour_utc FROM rollup_dirty_hours ORDER BY hour_utc")
        .map_err(|error| AppError::Database(format!("读取脏小时失败: {error}")))?;
    let hours = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(|error| AppError::Database(format!("读取脏小时失败: {error}")))?;
    drop(statement);

    for hour in &hours {
        let end = hour.saturating_add(3600);
        conn.execute(
            "DELETE FROM usage_hourly_rollups WHERE hour_utc = ?1",
            [hour],
        )
        .map_err(|error| AppError::Database(format!("重算脏小时失败: {error}")))?;
        let totals = aggregate_logs(
            conn,
            "WHERE created_at >= ?1 AND created_at < ?2",
            params![hour, end],
        )?;
        insert_totals(conn, totals)?;
        conn.execute("DELETE FROM rollup_dirty_hours WHERE hour_utc = ?1", [hour])
            .map_err(|error| AppError::Database(format!("清除脏小时失败: {error}")))?;
    }
    Ok(hours.len() as u64)
}

pub(crate) fn prune_hourly_rollups_before(
    conn: &Connection,
    cutoff_hour_utc: i64,
) -> Result<u64, AppError> {
    let mut deleted = 0u64;
    loop {
        let batch = conn
            .execute(
                "DELETE FROM usage_hourly_rollups WHERE rowid IN (
                     SELECT rowid FROM usage_hourly_rollups WHERE hour_utc < ?1 LIMIT ?2
                 )",
                params![cutoff_hour_utc, PRUNE_BATCH_LIMIT],
            )
            .map_err(|error| AppError::Database(format!("清理小时聚合失败: {error}")))?;
        deleted += batch as u64;
        if batch == 0 {
            break;
        }
    }
    Ok(deleted)
}

#[allow(dead_code)]
pub(crate) fn increment_guard_counter(
    conn: &Connection,
    local_date: &str,
    blocked: i64,
    allowed: i64,
    allowed_micro_usd: i64,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO guard_counters (
             local_date, blocked_count, allowed_count, allowed_micro_usd
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(local_date) DO UPDATE SET
             blocked_count = blocked_count + excluded.blocked_count,
             allowed_count = allowed_count + excluded.allowed_count,
             allowed_micro_usd = allowed_micro_usd + excluded.allowed_micro_usd",
        params![local_date, blocked, allowed, allowed_micro_usd],
    )
    .map_err(|error| AppError::Database(format!("更新守卫计数失败: {error}")))?;
    Ok(())
}

/// One settled proxy request. The credential itself is never a field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySettlement {
    pub request_id: String,
    pub provider_id: String,
    pub app_type: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub status_code: i64,
    pub created_at: i64,
    pub billing_identity: String,
    pub key_fingerprint: Option<String>,
    pub session_id: Option<String>,
    pub end_reason: String,
    pub error_message: Option<String>,
    pub guard_released_by: Option<String>,
    pub held_ms: Option<i64>,
}

pub(crate) fn insert_proxy_settlement(
    conn: &mut Connection,
    row: &ProxySettlement,
) -> Result<(), AppError> {
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| AppError::Database(format!("结算代理请求失败: {error}")))?;
    let inserted = transaction
        .execute(
            "INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model,
                 input_tokens, output_tokens, total_cost_usd, latency_ms, status_code,
                 session_id, is_streaming, created_at, data_source,
                 billing_identity, key_fingerprint, end_reason, cost_source, error_message,
                 guard_released_by, held_ms
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, '0', 0, ?7, ?8, 1, ?9, 'proxy',
                 ?10, ?11, ?12, 'metered', ?13, ?14, ?15
             ) ON CONFLICT(request_id) DO NOTHING",
            params![
                row.request_id,
                row.provider_id,
                row.app_type,
                row.model,
                row.input_tokens,
                row.output_tokens,
                row.status_code,
                row.session_id,
                row.created_at,
                row.billing_identity,
                row.key_fingerprint,
                row.end_reason,
                row.error_message,
                row.guard_released_by,
                row.held_ms,
            ],
        )
        .map_err(|error| AppError::Database(format!("结算代理请求失败: {error}")))?;
    if inserted == 1 {
        upsert_hourly_delta(
            &transaction,
            &HourlyLogDelta {
                created_at: row.created_at,
                app_type: row.app_type.clone(),
                provider_id: row.provider_id.clone(),
                model: row.model.clone(),
                billing_identity: row.billing_identity.clone(),
                data_source: "proxy".to_string(),
                status_code: row.status_code,
                end_reason: Some(row.end_reason.clone()),
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                total_cost_usd: "0".to_string(),
                latency_ms: 0,
            },
        )?;
    }
    transaction
        .commit()
        .map_err(|error| AppError::Database(format!("结算代理请求失败: {error}")))?;
    Ok(())
}

impl Database {
    pub fn detail_retention_days(&self) -> Result<i64, AppError> {
        match self.get_setting(DETAIL_RETENTION_SETTING_KEY)? {
            None => Ok(DETAIL_RETENTION_DAYS_DEFAULT),
            Some(raw) => match raw.parse::<i64>() {
                Ok(days)
                    if (DETAIL_RETENTION_DAYS_MIN..=DETAIL_RETENTION_DAYS_MAX).contains(&days) =>
                {
                    Ok(days)
                }
                _ => {
                    log::warn!(
                        "invalid detail retention setting, using {DETAIL_RETENTION_DAYS_DEFAULT}"
                    );
                    Ok(DETAIL_RETENTION_DAYS_DEFAULT)
                }
            },
        }
    }

    pub fn set_detail_retention_days(&self, days: i64) -> Result<(), AppError> {
        if !(DETAIL_RETENTION_DAYS_MIN..=DETAIL_RETENTION_DAYS_MAX).contains(&days) {
            return Err(AppError::invalid_argument(
                format!(
                    "Retention days must be an integer from {DETAIL_RETENTION_DAYS_MIN} to {DETAIL_RETENTION_DAYS_MAX}."
                ),
                "retentionDays",
            ));
        }
        self.set_setting(DETAIL_RETENTION_SETTING_KEY, &days.to_string())
    }
}

fn aggregate_logs(
    conn: &Connection,
    where_sql: &str,
    query_params: impl rusqlite::Params,
) -> Result<HashMap<HourKey, HourTotals>, AppError> {
    let sql = format!(
        "SELECT created_at, app_type, provider_id, model,
                COALESCE(billing_identity, ''),
                COALESCE(data_source, 'proxy'),
                status_code, end_reason,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_cost_usd, latency_ms
         FROM proxy_request_logs
         {where_sql}"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?;
    let mut rows = statement
        .query(query_params)
        .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?;
    let mut totals = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?
    {
        contribute(&mut totals, &delta_from_row(row)?)?;
    }
    Ok(totals)
}

fn delta_from_row(row: &Row<'_>) -> Result<HourlyLogDelta, AppError> {
    Ok(HourlyLogDelta {
        created_at: row
            .get(0)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        app_type: row
            .get(1)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        provider_id: row
            .get(2)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        model: row
            .get(3)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        billing_identity: row
            .get(4)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        data_source: row
            .get(5)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        status_code: row
            .get(6)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        end_reason: row
            .get::<_, Option<String>>(7)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?
            .map(|reason| {
                if reason.to_ascii_lowercase().starts_with("guard_hold") {
                    "guard_hold".to_string()
                } else {
                    reason
                }
            }),
        input_tokens: row
            .get(8)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        output_tokens: row
            .get(9)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        cache_read_tokens: row
            .get(10)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        cache_creation_tokens: row
            .get(11)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        total_cost_usd: row
            .get(12)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
        latency_ms: row
            .get(13)
            .map_err(|error| AppError::Database(format!("读取请求明细失败: {error}")))?,
    })
}

fn contribute(
    totals: &mut HashMap<HourKey, HourTotals>,
    delta: &HourlyLogDelta,
) -> Result<(), AppError> {
    // Gate decisions are audit-only, including legacy Hold sub-reasons.
    // Apply to live writes, dirty-hour maintenance and full rebuilds.
    if delta.end_reason.as_deref().is_some_and(|reason| {
        reason == "guard_deny" || reason.to_ascii_lowercase().starts_with("guard_hold")
    }) {
        return Ok(());
    }
    let key = HourKey {
        hour_utc: hour_bucket_utc(delta.created_at),
        app_type: delta.app_type.clone(),
        provider_id: delta.provider_id.clone(),
        model: delta.model.clone(),
        billing_identity: delta.billing_identity.clone(),
        data_source: delta.data_source.clone(),
    };
    let entry = totals.entry(key).or_default();
    entry.request_count = checked_add(entry.request_count, 1)?;
    if (200..300).contains(&delta.status_code) {
        entry.success_count = checked_add(entry.success_count, 1)?;
    }
    match delta.end_reason.as_deref() {
        Some(END_REASON_BLOCKED) => {
            entry.blocked_count = checked_add(entry.blocked_count, 1)?;
        }
        Some(END_REASON_CLIENT_ABORT) => {
            entry.abort_count = checked_add(entry.abort_count, 1)?;
        }
        _ => {}
    }
    entry.input_tokens = checked_add(entry.input_tokens, delta.input_tokens)?;
    entry.output_tokens = checked_add(entry.output_tokens, delta.output_tokens)?;
    entry.cache_read_tokens = checked_add(entry.cache_read_tokens, delta.cache_read_tokens)?;
    entry.cache_creation_tokens =
        checked_add(entry.cache_creation_tokens, delta.cache_creation_tokens)?;
    entry.cost_micro_usd = checked_add(
        entry.cost_micro_usd,
        usd_text_to_micro(&delta.total_cost_usd),
    )?;
    entry.latency_ms_sum = checked_add(entry.latency_ms_sum, delta.latency_ms)?;
    Ok(())
}

fn insert_totals(conn: &Connection, totals: HashMap<HourKey, HourTotals>) -> Result<(), AppError> {
    if totals.is_empty() {
        return Ok(());
    }
    let mut statement = conn
        .prepare(
            "INSERT INTO usage_hourly_rollups (
                 hour_utc, app_type, provider_id, model, billing_identity, data_source,
                 request_count, success_count, blocked_count, abort_count,
                 input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                 cost_micro_usd, latency_ms_sum
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
             )
             ON CONFLICT(hour_utc, app_type, provider_id, model, billing_identity, data_source)
             DO UPDATE SET
                 request_count = request_count + excluded.request_count,
                 success_count = success_count + excluded.success_count,
                 blocked_count = blocked_count + excluded.blocked_count,
                 abort_count = abort_count + excluded.abort_count,
                 input_tokens = input_tokens + excluded.input_tokens,
                 output_tokens = output_tokens + excluded.output_tokens,
                 cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                 cache_creation_tokens = cache_creation_tokens + excluded.cache_creation_tokens,
                 cost_micro_usd = cost_micro_usd + excluded.cost_micro_usd,
                 latency_ms_sum = latency_ms_sum + excluded.latency_ms_sum",
        )
        .map_err(|error| AppError::Database(format!("写入小时聚合失败: {error}")))?;
    for (key, totals) in totals {
        statement
            .execute(params![
                key.hour_utc,
                key.app_type,
                key.provider_id,
                key.model,
                key.billing_identity,
                key.data_source,
                totals.request_count,
                totals.success_count,
                totals.blocked_count,
                totals.abort_count,
                totals.input_tokens,
                totals.output_tokens,
                totals.cache_read_tokens,
                totals.cache_creation_tokens,
                totals.cost_micro_usd,
                totals.latency_ms_sum,
            ])
            .map_err(|error| AppError::Database(format!("写入小时聚合失败: {error}")))?;
    }
    Ok(())
}

fn checked_add(left: i64, right: i64) -> Result<i64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Database("hourly rollup counter overflow".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micro_conversion_is_exact_for_decimal_dollars() {
        assert_eq!(usd_text_to_micro("0.10"), 100_000);
        assert_eq!(usd_text_to_micro("0.000001"), 1);
        assert_eq!(usd_text_to_micro("1.234567"), 1_234_567);
        assert_eq!(usd_text_to_micro("0.07"), 70_000);
        assert_eq!(usd_text_to_micro("0"), 0);
        assert_eq!(usd_text_to_micro("not-a-number"), 0);
    }

    #[test]
    fn whole_hour_offset_is_exact_and_half_hour_offset_is_approximate() {
        let hour = hour_bucket_utc(1_782_000_000);
        let exact = local_day_for_hour(hour, 8 * 3600).unwrap();
        assert!(!exact.approximate);
        let half = local_day_for_hour(hour, 5 * 3600 + 30 * 60).unwrap();
        assert!(half.approximate);
    }

    fn insert_hour(conn: &Connection, hour_utc: i64, model: &str) {
        conn.execute(
            "INSERT INTO usage_hourly_rollups (
                 hour_utc, app_type, provider_id, model, billing_identity, data_source,
                 request_count, cost_micro_usd
             ) VALUES (?1, 'claude', 'anthropic', ?2, '', 'proxy', 1, 1)",
            params![hour_utc, model],
        )
        .unwrap();
    }

    #[test]
    fn prune_respects_a_non_default_hourly_retention() -> Result<(), AppError> {
        let db = Database::memory()?;
        let conn = lock_conn!(db.conn);
        let now = 1_800_000_000_i64;
        let recent = hour_bucket_utc(now - 20 * 86_400);
        let middle = hour_bucket_utc(now - 200 * 86_400);
        let old = hour_bucket_utc(now - 500 * 86_400);
        insert_hour(&conn, recent, "recent");
        insert_hour(&conn, middle, "middle");
        insert_hour(&conn, old, "old");

        let deleted = prune_hourly_rollups(&conn, now, 730).unwrap();
        assert_eq!(deleted, 0, "730 keeps a 500-day row for a later Pro build");
        assert_eq!(hourly_row_count(&conn), 3);

        let deleted = prune_hourly_rollups(&conn, now, HOURLY_RETENTION_DAYS_DEFAULT).unwrap();
        assert_eq!(deleted, 1);
        assert!(hour_exists(&conn, recent));
        assert!(hour_exists(&conn, middle));
        assert!(!hour_exists(&conn, old));

        let deleted = prune_hourly_rollups(&conn, now, 30).unwrap();
        assert_eq!(deleted, 1);
        assert!(hour_exists(&conn, recent));
        assert!(!hour_exists(&conn, middle));
        assert_eq!(
            RetentionPolicy::community(DETAIL_RETENTION_DAYS_DEFAULT).hourly_days,
            HOURLY_RETENTION_DAYS_DEFAULT
        );
        Ok(())
    }

    fn hourly_row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM usage_hourly_rollups", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn hour_exists(conn: &Connection, hour_utc: i64) -> bool {
        conn.query_row(
            "SELECT 1 FROM usage_hourly_rollups WHERE hour_utc = ?1",
            [hour_utc],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some()
    }

    #[test]
    fn guard_counter_upserts_one_local_day() -> Result<(), AppError> {
        let db = Database::memory()?;
        let conn = lock_conn!(db.conn);
        increment_guard_counter(&conn, "2026-10-25", 1, 2, 50).unwrap();
        increment_guard_counter(&conn, "2026-10-25", 3, 4, 7).unwrap();
        let (blocked, allowed, micro): (i64, i64, i64) = conn
            .query_row(
                "SELECT blocked_count, allowed_count, allowed_micro_usd
                 FROM guard_counters WHERE local_date = '2026-10-25'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((blocked, allowed, micro), (4, 6, 57));
        let other: Option<i64> = conn
            .query_row(
                "SELECT blocked_count FROM guard_counters WHERE local_date = '2026-10-26'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(other, None);
        Ok(())
    }
}
