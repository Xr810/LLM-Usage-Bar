//! QA-12 fixtures for the v29 hourly rollup (BE-15).
//!
//! The first backfill source is `proxy_request_logs` only. `usage_events` is
//! left in place and is not summed into the hourly table.

use chrono::{FixedOffset, NaiveDate, TimeZone};
use rusqlite::params;

use super::dao::hourly_rollup::{
    backfill_dirty_hours, hour_bucket_utc, local_day_for_hour, mark_rollup_hour_dirty,
    prune_hourly_rollups, rebuild_usage_hourly_rollups, usd_text_to_micro, RetentionPolicy,
    DETAIL_RETENTION_DAYS_DEFAULT, HEATMAP_MAX_WEEKS, HOURLY_RETENTION_DAYS_DEFAULT,
    PRUNE_BATCH_LIMIT,
};
use super::{lock_conn, Database};
use crate::error::AppError;

#[test]
fn heatmap_window_is_52_weeks_for_both_tiers() {
    assert_eq!(HEATMAP_MAX_WEEKS, 52);
    assert_eq!(
        super::dao::hourly_rollup::heatmap_max_weeks_for_tier("free"),
        52
    );
    assert_eq!(
        super::dao::hourly_rollup::heatmap_max_weeks_for_tier("pro"),
        52
    );
}

#[test]
fn timezone_fixtures_cover_exact_half_hour_and_us_eastern_dst() {
    let utc_hour = hour_bucket_utc(
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 25, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp(),
    );
    let plus_eight = local_day_for_hour(utc_hour, 8 * 3_600).unwrap();
    assert_eq!(
        plus_eight.date,
        NaiveDate::from_ymd_opt(2026, 10, 25).unwrap()
    );
    assert!(!plus_eight.approximate);

    let india = local_day_for_hour(utc_hour, 5 * 3_600 + 30 * 60).unwrap();
    assert_eq!(india.date, NaiveDate::from_ymd_opt(2026, 10, 25).unwrap());
    assert!(india.approximate);

    let est = FixedOffset::west_opt(5 * 3_600).unwrap();
    let edt = FixedOffset::west_opt(4 * 3_600).unwrap();
    let before_spring = est
        .with_ymd_and_hms(2026, 3, 8, 1, 0, 0)
        .single()
        .unwrap()
        .timestamp();
    let after_spring = edt
        .with_ymd_and_hms(2026, 3, 8, 3, 0, 0)
        .single()
        .unwrap()
        .timestamp();
    let before_day = local_day_for_hour(hour_bucket_utc(before_spring), -5 * 3_600).unwrap();
    let after_day = local_day_for_hour(hour_bucket_utc(after_spring), -4 * 3_600).unwrap();
    assert_eq!(
        before_day.date,
        NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()
    );
    assert_eq!(after_day.date, NaiveDate::from_ymd_opt(2026, 3, 8).unwrap());
    assert!(!before_day.approximate);
    assert!(!after_day.approximate);
    assert_ne!(
        hour_bucket_utc(before_spring),
        hour_bucket_utc(after_spring),
        "spring-forward keeps the two local hours in different UTC buckets"
    );
}

#[test]
fn hourly_total_matches_proxy_logs_and_ignores_usage_events() -> Result<(), AppError> {
    let db = Database::memory()?;
    let conn = lock_conn!(db.conn);
    conn.execute(
        "INSERT INTO usage_providers (
             id, name, billing_kind, product_group_id, token_sources,
             enabled, needs_review, created_at, updated_at
         ) VALUES ('history-provider', 'History', 'metered', 'history', '[\"proxy\"]', 1, 0, 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO usage_events (
             event_id, source, provider_id, product_group_id, occurred_at, model,
             input_tokens, output_tokens, total_cost_usd, cost_source, created_at
         ) VALUES (
             'old-event', 'proxy', 'history-provider', 'history', 1_600_000_000,
             'claude', 999, 999, '9.99', 'upstream', 1_600_000_000
         )",
        [],
    )?;
    for (request_id, cost, billing) in [("a", "0.10", None), ("b", "0.000001", Some(""))] {
        conn.execute(
            "INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model, input_tokens, output_tokens,
                 total_cost_usd, latency_ms, status_code, created_at, data_source,
                 billing_identity
             ) VALUES (?1, 'anthropic', 'claude', 'claude', 1, 1, ?2, 5, 200, 1700000100, 'proxy', ?3)",
            params![request_id, cost, billing],
        )?;
    }
    rebuild_usage_hourly_rollups(&conn)?;

    let (rows, requests, micro): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(request_count), 0), COALESCE(SUM(cost_micro_usd), 0)
         FROM usage_hourly_rollups",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(rows, 1, "NULL and empty billing identity share one bucket");
    assert_eq!(requests, 2);
    assert_eq!(
        micro,
        usd_text_to_micro("0.10") + usd_text_to_micro("0.000001")
    );
    let events: i64 = conn.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))?;
    assert_eq!(events, 1, "usage_events stays untouched");
    Ok(())
}

#[test]
fn dirty_hour_rebuild_replaces_the_bucket_instead_of_adding() -> Result<(), AppError> {
    let db = Database::memory()?;
    let conn = lock_conn!(db.conn);
    conn.execute(
        "INSERT INTO proxy_request_logs (
             request_id, provider_id, app_type, model, total_cost_usd, latency_ms,
             status_code, created_at
         ) VALUES ('priced', 'anthropic', 'claude', 'claude', '0.10', 1, 200, 1700000100)",
        [],
    )?;
    rebuild_usage_hourly_rollups(&conn)?;
    conn.execute(
        "UPDATE proxy_request_logs SET total_cost_usd = '0.20' WHERE request_id = 'priced'",
        [],
    )?;
    mark_rollup_hour_dirty(&conn, 1_700_000_100)?;
    assert_eq!(backfill_dirty_hours(&conn)?, 1);

    let micro: i64 = conn.query_row(
        "SELECT cost_micro_usd FROM usage_hourly_rollups",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(micro, usd_text_to_micro("0.20"));
    let pending: i64 = conn.query_row("SELECT COUNT(*) FROM rollup_dirty_hours", [], |row| {
        row.get(0)
    })?;
    assert_eq!(pending, 0);
    Ok(())
}

#[test]
fn retention_boundaries_follow_the_policy_argument() -> Result<(), AppError> {
    let db = Database::memory()?;
    assert_eq!(
        RetentionPolicy::community(DETAIL_RETENTION_DAYS_DEFAULT).hourly_days,
        HOURLY_RETENTION_DAYS_DEFAULT
    );
    let now = 1_800_000_000_i64;
    let cutoff = super::dao::hourly_rollup::hourly_retention_cutoff(now, 400)?;
    {
        let conn = lock_conn!(db.conn);
        for (hour, model) in [(cutoff, "kept"), (cutoff - 3_600, "dropped")] {
            conn.execute(
                "INSERT INTO usage_hourly_rollups (
                     hour_utc, app_type, provider_id, model, billing_identity, data_source,
                     request_count
                 ) VALUES (?1, 'claude', 'anthropic', ?2, '', 'proxy', 1)",
                params![hour, model],
            )?;
        }
        let deleted = prune_hourly_rollups(&conn, now, 400)?;
        assert_eq!(deleted, 1);
        let kept: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage_hourly_rollups WHERE model = 'kept'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(kept, 1);
    }

    db.set_detail_retention_days(365)?;
    let old = chrono::Utc::now().timestamp() - 120 * 86_400;
    {
        let conn = lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model, total_cost_usd, latency_ms,
                 status_code, created_at
             ) VALUES ('detail-120', 'anthropic', 'claude', 'claude', '0.01', 1, 200, ?1)",
            [old],
        )?;
    }
    assert_eq!(db.rollup_and_prune_configured()?, 0);
    db.set_detail_retention_days(90)?;
    assert_eq!(db.rollup_and_prune_configured()?, 1);
    Ok(())
}

#[test]
fn hourly_prune_batches_past_the_row_limit() -> Result<(), AppError> {
    let db = Database::memory()?;
    let conn = lock_conn!(db.conn);
    let rows = PRUNE_BATCH_LIMIT + 1;
    for index in 0..rows {
        conn.execute(
            "INSERT INTO usage_hourly_rollups (
                 hour_utc, app_type, provider_id, model, billing_identity, data_source,
                 request_count
             ) VALUES (?1, 'claude', 'anthropic', 'claude', '', 'proxy', 1)",
            [index * 3_600],
        )?;
    }
    let deleted = prune_hourly_rollups(&conn, 1_800_000_000, 1)?;
    assert_eq!(deleted, rows as u64);
    let left: i64 = conn.query_row("SELECT COUNT(*) FROM usage_hourly_rollups", [], |row| {
        row.get(0)
    })?;
    assert_eq!(left, 0);
    Ok(())
}
