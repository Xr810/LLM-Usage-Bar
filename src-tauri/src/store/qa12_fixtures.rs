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
fn guard_deny_is_audit_only_across_usage_queries_and_retention() -> Result<(), AppError> {
    use crate::policy::{LicenseBuild, LicenseStatus, LicenseTier};
    use crate::usage::{aggregation, trend::query_usage_trend_at};
    use serde_json::json;

    let db = Database::memory()?;
    let now = chrono::Utc::now();
    let at = chrono::Local
        .from_local_datetime(
            &chrono::Local::now()
                .date_naive()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
        .timestamp();
    let start = at - 86400;
    let end = at + 86400;
    let mut license = LicenseStatus::community_free();
    license.build = LicenseBuild::Official;
    license.tier = LicenseTier::Pro;
    {
        let conn = lock_conn!(db.conn);
        // Distinct ordinary, Hold and pre-existing blocked controls. D2 must
        // preserve all three, including Hold's existing request count.
        for (id, reason, status, input, output, cost) in [
            ("used", None, 200, 13, 7, "0.25"),
            ("held", Some("guard_hold"), 429, 0, 0, "0"),
            ("blocked", Some("blocked"), 403, 0, 0, "0"),
        ] {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                     request_id, provider_id, app_type, model, input_tokens,
                     output_tokens, total_cost_usd, latency_ms, status_code,
                     created_at, end_reason, project_alias, client_tag_id
                 ) VALUES (?1, 'p', 'claude', 'm', ?2, ?3, ?4, 11, ?5, ?6, ?7, 'project', 'client')",
                params![id, input, output, cost, status, at, reason],
            )?;
        }
        conn.execute_batch(
            "INSERT INTO usage_providers (id, name, billing_kind, product_group_id,
                 token_sources, enabled, needs_review, created_at, updated_at)
             VALUES ('p', 'Provider', 'metered', 'product', '[\"proxy\"]', 1, 0, 1, 1);",
        )?;
        conn.execute(
            "INSERT INTO usage_events (event_id, source, provider_id, product_group_id,
                 occurred_at, model, input_tokens, output_tokens, total_cost_usd,
                 cost_source, created_at)
             VALUES ('event', 'proxy', 'p', 'product', ?1, 'm', 13, 7, '0.25', 'upstream', ?1)",
            [at],
        )?;
        rebuild_usage_hourly_rollups(&conn)?;
    }
    let snapshot = || -> Result<serde_json::Value, AppError> {
        let mut trends = Vec::new();
        for group in [None, Some("provider"), Some("project"), Some("client")] {
            trends.push(query_usage_trend_at(&db, json!({
                "tz": "Asia/Hong_Kong",
                "from": chrono::DateTime::from_timestamp(start, 0).unwrap().with_timezone(&chrono_tz::Asia::Hong_Kong).format("%Y-%m-%d").to_string(),
                "to": chrono::DateTime::from_timestamp(end, 0).unwrap().with_timezone(&chrono_tz::Asia::Hong_Kong).format("%Y-%m-%d").to_string(),
                "granularity": "day", "groupBy": group
            }), &license, now)?);
        }
        Ok(json!({
            "summary": db.get_usage_summary(None, None, None, None, None)?,
            "today": db.get_usage_summary(Some(at - 3600), Some(end), None, None, None)?,
            "byApp": db.get_usage_summary_by_app(None, None, None, None)?,
            "hourly": db.get_daily_trends(Some(at - 3600), Some(at + 3600), None, None, None)?,
            "daily": db.get_daily_trends(Some(start), Some(end), None, None, None)?,
            "providers": db.get_provider_stats(None, None, None, None, None)?,
            "models": db.get_model_stats(None, None, None, None, None)?,
            "sources": super::usage_stats::get_data_source_breakdown(&db)?,
            "limits": db.check_provider_limits("p", "claude")?,
            "sessions": crate::usage::session_list::list_sessions_at(&db, at, false, None, None)?,
            "trends": trends,
            "eventModels": aggregation::aggregate_model_usage(&db, start, end)?,
            "agents": aggregation::aggregate_agent_usage(&db, start, end)?,
            "eventProviders": aggregation::aggregate_enabled_provider_trend(&db, start, end)?.1
        }))
    };
    let before = snapshot()?;
    assert_eq!(before["summary"]["totalRequests"], 3);
    assert_eq!(before["summary"]["totalInputTokens"], 13);
    assert_eq!(before["summary"]["totalOutputTokens"], 7);
    assert_eq!(before["limits"]["dailyUsage"], "0.250000");
    let today_before = aggregation::aggregate_provider_account_range(&db, "p", start, end)?;
    assert_eq!(today_before.event_count, 1);
    assert_eq!(today_before.total_cost_usd.as_deref(), Some("0.25"));
    {
        let conn = lock_conn!(db.conn);
        // Deliberately nonzero imported data proves exclusion by end_reason,
        // not an accidental zero-cost/token sum or HTTP-error exclusion.
        for status in [200, 451] {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                     request_id, provider_id, app_type, model, input_tokens,
                     output_tokens, total_cost_usd, latency_ms, status_code,
                     created_at, end_reason, project_alias, client_tag_id
                 ) VALUES (?1, 'p', 'claude', 'm', 101, 203, '17.5', 999, ?2, ?3,
                           'guard_deny', 'project', 'client')",
                params![format!("denied-{status}"), status, at],
            )?;
        }
        rebuild_usage_hourly_rollups(&conn)?;
        mark_rollup_hour_dirty(&conn, at)?;
        backfill_dirty_hours(&conn)?;
    }
    assert_eq!(snapshot()?, before, "Deny must not change any usage view");
    assert_eq!(
        aggregation::aggregate_provider_account_range(&db, "p", start, end)?,
        today_before,
        "tray today totals and guard spend consume the event ledger, not audit rows"
    );
    assert_eq!(
        db.get_request_logs(&Default::default(), 1, 20)?.total,
        5,
        "Deny must remain visible in the audit list"
    );
    // Move all details into the past, then exercise daily archival and query
    // the same all-time totals: denied counts/tokens/cost must not reappear.
    {
        let conn = lock_conn!(db.conn);
        conn.execute(
            "UPDATE proxy_request_logs SET created_at = ?1",
            [at - 3 * 86400],
        )?;
    }
    assert_eq!(db.rollup_and_prune(1)?, 5);
    let summary = db.get_usage_summary(None, None, None, None, None)?;
    assert_eq!(summary.total_requests, 3);
    assert_eq!(summary.total_input_tokens, 13);
    assert_eq!(summary.total_output_tokens, 7);
    assert_eq!(summary.total_cost.parse::<f64>().unwrap(), 0.25);
    Ok(())
}

#[test]
fn injected_retention_survives_startup_and_periodic_maintenance() -> Result<(), AppError> {
    struct PrivateRetention;
    impl super::RetentionPolicyProvider for PrivateRetention {
        fn retention_policy(&self, detail_days: i64) -> RetentionPolicy {
            RetentionPolicy {
                detail_days,
                hourly_days: 730,
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    {
        let db = Database::init_at(&path)?;
        let conn = lock_conn!(db.conn);
        for days in [100, 500, 800] {
            conn.execute(
                "INSERT INTO usage_hourly_rollups
                 (hour_utc, app_type, provider_id, model, billing_identity, data_source, request_count)
                 VALUES (?1, 'claude', 'anthropic', 'claude', '', 'proxy', 1)",
                [hour_bucket_utc(chrono::Utc::now().timestamp() - days * 86_400)],
            )?;
        }
    }
    let db = Database::init_at_with_retention_policy(&path, Box::new(PrivateRetention))?;
    db.periodic_backup_if_needed()?;
    let count =
        |db: &Database| -> Result<i64, AppError> {
            Ok(lock_conn!(db.conn).query_row(
                "SELECT COUNT(*) FROM usage_hourly_rollups",
                [],
                |r| r.get(0),
            )?)
        };
    assert_eq!(count(&db)?, 2);
    assert_eq!(db.raise_retention_high_water_days(400)?, 730);
    drop(db);
    let community = Database::init_at(&path)?;
    community.rollup_and_prune_configured()?;
    assert_eq!(count(&community)?, 2);
    assert_eq!(
        community
            .get_setting("retention_high_water_days")?
            .as_deref(),
        Some("730")
    );
    Ok(())
}

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
