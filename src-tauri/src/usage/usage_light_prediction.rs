//! Append-only-at-runtime prediction/outcome writer for future self-calibration.
//!
//! Nothing reads these rows to change a verdict yet. This pass only records live
//! predictions and resolves completed subscription windows so a later calibration
//! pass has honest local outcome data instead of reconstructing past decisions.

use super::domain::QuotaSnapshot;
use super::status::{PaceBasis, SourceClassification, UsageStatus};
use super::subscription_pace::{FIVE_HOUR_WINDOW_KIND, SEVEN_DAY_WINDOW_KIND};
use crate::error::AppError;
use crate::store::Database;
use chrono::DateTime;
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;

pub(crate) const SHARED_DAILY_BUDGET_PROVIDER_ID: &str = "__shared_daily_budget__";
const HEARTBEAT_SECONDS: i64 = 15 * 60;

pub(crate) fn record_live_prediction(
    db: &Database,
    provider_id: &str,
    window_kind: &str,
    window_resets_at: i64,
    predicted_at: i64,
    classification: &SourceClassification,
) -> Result<bool, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let latest = conn
        .query_row(
            "SELECT status, predicted_at
             FROM usage_light_predictions
             WHERE provider_id = ?1 AND window_kind = ?2 AND window_resets_at = ?3
             ORDER BY predicted_at DESC, rowid DESC
             LIMIT 1",
            params![provider_id, window_kind, window_resets_at],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let status = status_text(classification.status);
    let should_write = latest.is_none_or(|(latest_status, latest_at)| {
        latest_status != status
            || predicted_at
                .checked_sub(latest_at)
                .is_some_and(|elapsed| elapsed >= HEARTBEAT_SECONDS)
    });
    if !should_write {
        return Ok(false);
    }

    let prediction_id = uuid::Uuid::new_v4().to_string();
    let used_percent = classification
        .used_percent
        .as_deref()
        .or(classification.consumed_percent.as_deref());
    conn.execute(
        "INSERT INTO usage_light_predictions (
            prediction_id, provider_id, window_kind, window_resets_at,
            predicted_at, status, used_percent, headroom_ratio, burn_rate,
            pace_basis, rhythm_adjustment, source
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'live')",
        params![
            prediction_id,
            provider_id,
            window_kind,
            window_resets_at,
            predicted_at,
            status,
            used_percent,
            classification
                .headroom_ratio
                .map(|value| value.normalize().to_string()),
            classification
                .burn_rate_per_second
                .map(|value| value.normalize().to_string()),
            pace_basis_text(classification.pace_basis),
            classification
                .rhythm_adjustment
                .map(|value| value.normalize().to_string()),
        ],
    )?;
    Ok(true)
}

pub(crate) fn resolve_snapshot_rollovers(
    conn: &Connection,
    previous: &QuotaSnapshot,
    current: &QuotaSnapshot,
) -> Result<usize, AppError> {
    if current.provider_id != previous.provider_id || current.fetched_at <= previous.fetched_at {
        return Ok(0);
    }
    let mut resolved = 0_usize;
    for (kind, previous_reset, current_reset) in [
        (
            FIVE_HOUR_WINDOW_KIND,
            previous.five_hour_resets_at.as_deref(),
            current.five_hour_resets_at.as_deref(),
        ),
        (
            SEVEN_DAY_WINDOW_KIND,
            previous.seven_day_resets_at.as_deref(),
            current.seven_day_resets_at.as_deref(),
        ),
    ] {
        let (Some(previous_reset), Some(current_reset)) = (previous_reset, current_reset) else {
            continue;
        };
        if previous_reset == current_reset {
            continue;
        }
        resolved = resolved
            .checked_add(resolve_window_instance(
                conn,
                &current.provider_id,
                kind,
                previous_reset,
                current.fetched_at,
            )?)
            .ok_or_else(|| {
                AppError::Database("prediction resolution count overflow".to_string())
            })?;
    }
    Ok(resolved)
}

fn resolve_window_instance(
    conn: &Connection,
    provider_id: &str,
    window_kind: &str,
    reset_raw: &str,
    resolved_at: i64,
) -> Result<usize, AppError> {
    let Some(reset_timestamp) = DateTime::parse_from_rfc3339(reset_raw)
        .ok()
        .map(|value| value.timestamp())
    else {
        return Ok(0);
    };
    let (used_column, reset_column) = match window_kind {
        FIVE_HOUR_WINDOW_KIND => ("five_hour_utilization_percent", "five_hour_resets_at"),
        SEVEN_DAY_WINDOW_KIND => ("seven_day_utilization_percent", "seven_day_resets_at"),
        _ => return Ok(0),
    };
    let sql = format!(
        "SELECT {used_column}
         FROM quota_snapshots
         WHERE provider_id = ?1 AND {reset_column} = ?2"
    );
    let mut statement = conn.prepare(&sql)?;
    let values = statement.query_map(params![provider_id, reset_raw], |row| {
        row.get::<_, Option<String>>(0)
    })?;
    let mut peak = None;
    for value in values {
        let Some(value) = value?
            .as_deref()
            .and_then(|raw| Decimal::from_str_exact(raw.trim()).ok())
        else {
            continue;
        };
        peak = Some(peak.map_or(value, |current: Decimal| current.max(value)));
    }
    drop(statement);
    let outcome = if peak.is_some_and(|value| value >= Decimal::from(99_i64)) {
        "exhausted"
    } else {
        "survived"
    };
    let peak = peak.map(|value| value.normalize().to_string());
    conn.execute(
        "UPDATE usage_light_predictions
         SET outcome = ?4, outcome_peak_used = ?5, outcome_resolved_at = ?6
         WHERE provider_id = ?1 AND window_kind = ?2 AND window_resets_at = ?3
           AND outcome IS NULL",
        params![
            provider_id,
            window_kind,
            reset_timestamp,
            outcome,
            peak,
            resolved_at,
        ],
    )
    .map_err(AppError::from)
}

pub(crate) fn prune_resolved_predictions(
    conn: &Connection,
    resolved_before: i64,
) -> Result<usize, AppError> {
    conn.execute(
        "DELETE FROM usage_light_predictions
         WHERE outcome IS NOT NULL
           AND outcome_resolved_at IS NOT NULL
           AND outcome_resolved_at < ?1",
        [resolved_before],
    )
    .map_err(AppError::from)
}

fn status_text(status: UsageStatus) -> &'static str {
    match status {
        UsageStatus::Green => "green",
        UsageStatus::Yellow => "yellow",
        UsageStatus::Red => "red",
        UsageStatus::Unknown => "unknown",
    }
}

fn pace_basis_text(basis: PaceBasis) -> &'static str {
    match basis {
        PaceBasis::Measured => "measured",
        PaceBasis::WindowAverage => "window_average",
        PaceBasis::Static => "static",
        PaceBasis::Idle => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use serde_json::json;

    fn classification(status: UsageStatus) -> SourceClassification {
        SourceClassification {
            status,
            used_percent: Some("50".to_string()),
            remaining_percent: Some("50".to_string()),
            consumed_percent: None,
            reason: None,
            burn_rate_per_second: Some(Decimal::new(1, 3)),
            projected_exhaust_at: Some(10_000),
            headroom_ratio: Some(Decimal::new(11, 1)),
            pace_basis: PaceBasis::Measured,
            rhythm_adjustment: Some(Decimal::new(8, 1)),
            flat_status: None,
        }
    }

    fn save_provider(db: &Database, provider_id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: provider_id.to_string(),
            name: provider_id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: Some("official".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();
    }

    fn snapshot(
        provider_id: &str,
        id: &str,
        fetched_at: i64,
        used: &str,
        reset: &str,
    ) -> QuotaSnapshot {
        QuotaSnapshot {
            snapshot_id: format!("{provider_id}-{id}"),
            provider_id: provider_id.to_string(),
            fetched_at,
            five_hour_utilization_percent: Some(used.to_string()),
            five_hour_resets_at: Some(reset.to_string()),
            seven_day_utilization_percent: None,
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: json!({}),
            created_at: fetched_at,
        }
    }

    fn row_count(db: &Database, provider_id: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM usage_light_predictions WHERE provider_id = ?1",
            [provider_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn cadence_keeps_every_status_change_and_throttles_heartbeats() {
        let db = Database::memory().unwrap();
        let reset = 20_000;

        assert!(record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            1_000,
            &classification(UsageStatus::Green),
        )
        .unwrap());
        assert!(!record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            1_100,
            &classification(UsageStatus::Green),
        )
        .unwrap());
        assert!(record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            1_101,
            &classification(UsageStatus::Yellow),
        )
        .unwrap());
        assert!(record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            1_102,
            &classification(UsageStatus::Red),
        )
        .unwrap());
        assert!(!record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            2_001,
            &classification(UsageStatus::Red),
        )
        .unwrap());
        assert!(record_live_prediction(
            &db,
            "provider",
            FIVE_HOUR_WINDOW_KIND,
            reset,
            2_002,
            &classification(UsageStatus::Red),
        )
        .unwrap());

        assert_eq!(row_count(&db, "provider"), 4);
    }

    #[test]
    fn rollover_resolves_exhausted_and_survived_but_not_current_instance() {
        let db = Database::memory().unwrap();
        let old_reset = "2026-01-01T05:00:00Z";
        let old_reset_timestamp = DateTime::parse_from_rfc3339(old_reset).unwrap().timestamp();
        let current_reset = "2026-01-01T10:00:00Z";
        let current_reset_timestamp = DateTime::parse_from_rfc3339(current_reset)
            .unwrap()
            .timestamp();

        for (provider_id, peak) in [("exhausted", "100"), ("survived", "60")] {
            save_provider(&db, provider_id);
            db.append_quota_success(&snapshot(provider_id, "old-low", 100, "20", old_reset))
                .unwrap();
            db.append_quota_success(&snapshot(provider_id, "old-peak", 200, peak, old_reset))
                .unwrap();
            record_live_prediction(
                &db,
                provider_id,
                FIVE_HOUR_WINDOW_KIND,
                old_reset_timestamp,
                250,
                &classification(UsageStatus::Red),
            )
            .unwrap();
            db.append_quota_success(&snapshot(provider_id, "current", 300, "0", current_reset))
                .unwrap();
            record_live_prediction(
                &db,
                provider_id,
                FIVE_HOUR_WINDOW_KIND,
                current_reset_timestamp,
                301,
                &classification(UsageStatus::Green),
            )
            .unwrap();
        }

        let conn = db.conn.lock().unwrap();
        for (provider_id, expected_outcome, expected_peak) in [
            ("exhausted", "exhausted", "100"),
            ("survived", "survived", "60"),
        ] {
            let resolved = conn
                .query_row(
                    "SELECT outcome, outcome_peak_used
                     FROM usage_light_predictions
                     WHERE provider_id = ?1 AND window_resets_at = ?2",
                    params![provider_id, old_reset_timestamp],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap();
            assert_eq!(
                resolved,
                (expected_outcome.to_string(), expected_peak.to_string())
            );
            let current_outcome = conn
                .query_row(
                    "SELECT outcome
                     FROM usage_light_predictions
                     WHERE provider_id = ?1 AND window_resets_at = ?2",
                    params![provider_id, current_reset_timestamp],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap();
            assert_eq!(current_outcome, None);
        }
    }
}
