use super::domain::QuotaSnapshot;
use super::rhythm::cached_quota_profile;
use super::status::{
    classify_subscription_window, PaceInput, PaceMeasurement, SourceClassification,
    SubscriptionThresholds, MIN_RATE_SPAN_SECONDS,
};
use crate::database::Database;
use crate::error::AppError;
use chrono::DateTime;
use rust_decimal::Decimal;

pub(crate) const FIVE_HOUR_WINDOW_KIND: &str = "five_hour";
pub(crate) const SEVEN_DAY_WINDOW_KIND: &str = "seven_day";

const FIVE_HOUR_WINDOW_SECONDS: i64 = 18_000;
const SEVEN_DAY_WINDOW_SECONDS: i64 = 604_800;
const INITIAL_QUOTA_RATE_LOOKBACK_SECONDS: i64 = 1_800;
const EXTENDED_QUOTA_RATE_LOOKBACK_SECONDS: i64 = 3_600;

pub(crate) fn classify_subscription_windows(
    db: &Database,
    provider_id: &str,
    snapshot: Option<&QuotaSnapshot>,
    now_timestamp: i64,
    thresholds: SubscriptionThresholds,
) -> Result<[SourceClassification; 2], AppError> {
    Ok([
        classify_window(
            db,
            provider_id,
            snapshot,
            FIVE_HOUR_WINDOW_KIND,
            now_timestamp,
            thresholds,
        )?,
        classify_window(
            db,
            provider_id,
            snapshot,
            SEVEN_DAY_WINDOW_KIND,
            now_timestamp,
            thresholds,
        )?,
    ])
}

fn classify_window(
    db: &Database,
    provider_id: &str,
    snapshot: Option<&QuotaSnapshot>,
    kind: &str,
    now_timestamp: i64,
    thresholds: SubscriptionThresholds,
) -> Result<SourceClassification, AppError> {
    let (used_percent, resets_at) = snapshot
        .map(|snapshot| quota_window_values(snapshot, kind))
        .unwrap_or((None, None));
    let measured = measured_quota_rate(db, provider_id, kind, resets_at, now_timestamp)?;
    let rhythm_profile = match cached_quota_profile(db, provider_id, kind, now_timestamp) {
        Ok(profile) => profile,
        Err(error) => {
            log::warn!(
                "Failed to build rhythm profile for {provider_id}/{kind}; using flat pace: {error}"
            );
            None
        }
    };

    Ok(classify_subscription_window(
        used_percent,
        PaceInput {
            now_timestamp,
            reset_timestamp: parse_reset_timestamp(resets_at),
            window_length_seconds: subscription_window_length(kind),
            measured_rate_per_second: measured.as_ref().map(|value| value.rate_per_second),
            measured_intervals: measured.map(|value| value.intervals).unwrap_or_default(),
            rhythm_profile,
        },
        thresholds,
    ))
}

fn measured_quota_rate(
    db: &Database,
    provider_id: &str,
    kind: &str,
    resets_at: Option<&str>,
    now_timestamp: i64,
) -> Result<Option<PaceMeasurement>, AppError> {
    let Some(window_length_seconds) = subscription_window_length(kind) else {
        return Ok(None);
    };
    let Some(reset_timestamp) = parse_reset_timestamp(resets_at) else {
        return Ok(None);
    };
    if reset_timestamp <= now_timestamp {
        return Ok(None);
    }
    let Some(window_start) = reset_timestamp.checked_sub(window_length_seconds) else {
        return Ok(None);
    };
    let lookbacks = [
        now_timestamp.checked_sub(INITIAL_QUOTA_RATE_LOOKBACK_SECONDS),
        now_timestamp.checked_sub(EXTENDED_QUOTA_RATE_LOOKBACK_SECONDS),
        Some(window_start),
    ];
    let mut previous_since = None;
    for since in lookbacks.into_iter().flatten() {
        if previous_since == Some(since) {
            continue;
        }
        previous_since = Some(since);
        let snapshots = db.quota_snapshots_since(provider_id, since)?;
        if let Some(measurement) = quota_measurement_from_snapshots(&snapshots, kind, now_timestamp)
        {
            return Ok(Some(measurement));
        }
    }
    Ok(None)
}

fn parse_reset_timestamp(raw: Option<&str>) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw?)
        .ok()
        .map(|value| value.timestamp())
}

pub(crate) fn subscription_window_length(kind: &str) -> Option<i64> {
    match kind {
        FIVE_HOUR_WINDOW_KIND => Some(FIVE_HOUR_WINDOW_SECONDS),
        SEVEN_DAY_WINDOW_KIND => Some(SEVEN_DAY_WINDOW_SECONDS),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn quota_rate_from_snapshots(
    snapshots: &[QuotaSnapshot],
    kind: &str,
    now_timestamp: i64,
) -> Option<Decimal> {
    quota_measurement_from_snapshots(snapshots, kind, now_timestamp)
        .map(|measurement| measurement.rate_per_second)
}

fn quota_measurement_from_snapshots(
    snapshots: &[QuotaSnapshot],
    kind: &str,
    now_timestamp: i64,
) -> Option<PaceMeasurement> {
    let mut total_used_delta = Decimal::ZERO;
    let mut total_span_seconds = 0_i64;
    let mut intervals = Vec::new();

    for pair in snapshots.windows(2) {
        let [earlier, later] = pair else {
            continue;
        };
        let Some(valid) = valid_quota_pair(earlier, later, kind, now_timestamp) else {
            continue;
        };
        total_used_delta = total_used_delta.checked_add(valid.used_delta)?;
        let span_seconds = valid.end_at.checked_sub(valid.start_at)?;
        total_span_seconds = total_span_seconds.checked_add(span_seconds)?;
        intervals.push((valid.start_at, valid.end_at));
    }

    if total_span_seconds < MIN_RATE_SPAN_SECONDS {
        return None;
    }
    Some(PaceMeasurement {
        rate_per_second: total_used_delta.checked_div(Decimal::from(total_span_seconds))?,
        intervals,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidQuotaPair {
    pub(crate) start_at: i64,
    pub(crate) end_at: i64,
    pub(crate) used_delta: Decimal,
}

pub(crate) fn valid_quota_pair(
    earlier: &QuotaSnapshot,
    later: &QuotaSnapshot,
    kind: &str,
    now_timestamp: i64,
) -> Option<ValidQuotaPair> {
    if earlier.fetched_at > now_timestamp || later.fetched_at > now_timestamp {
        return None;
    }
    let (earlier_used, earlier_reset) = quota_window_values(earlier, kind);
    let (later_used, later_reset) = quota_window_values(later, kind);
    let (Some(earlier_reset), Some(later_reset)) = (earlier_reset, later_reset) else {
        return None;
    };
    if earlier_reset != later_reset {
        return None;
    }
    let (Some(earlier_used), Some(later_used)) = (
        earlier_used.and_then(|value| Decimal::from_str_exact(value.trim()).ok()),
        later_used.and_then(|value| Decimal::from_str_exact(value.trim()).ok()),
    ) else {
        return None;
    };
    if later_used < earlier_used {
        return None;
    }
    let span_seconds = later.fetched_at.checked_sub(earlier.fetched_at)?;
    if span_seconds <= 0 {
        return None;
    }
    Some(ValidQuotaPair {
        start_at: earlier.fetched_at,
        end_at: later.fetched_at,
        used_delta: later_used.checked_sub(earlier_used)?,
    })
}

pub(crate) fn quota_window_values<'a>(
    snapshot: &'a QuotaSnapshot,
    kind: &str,
) -> (Option<&'a str>, Option<&'a str>) {
    match kind {
        FIVE_HOUR_WINDOW_KIND => (
            snapshot.five_hour_utilization_percent.as_deref(),
            snapshot.five_hour_resets_at.as_deref(),
        ),
        SEVEN_DAY_WINDOW_KIND => (
            snapshot.seven_day_utilization_percent.as_deref(),
            snapshot.seven_day_resets_at.as_deref(),
        ),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde_json::json;

    #[test]
    fn three_day_history_does_not_apply_rhythm_or_change_the_flat_verdict() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&UsageProviderInput {
            id: "cold-start-sub".to_string(),
            name: "Cold Start".to_string(),
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
        let now = 1_800_000_000_i64;
        let reset_timestamp = now + 3 * 3_600;
        let reset = DateTime::<Utc>::from_timestamp(reset_timestamp, 0)
            .unwrap()
            .to_rfc3339_opts(SecondsFormat::AutoSi, true);
        for hour in 0..=3 * 24_i64 {
            let fetched_at = now - (3 * 24 - hour) * 3_600;
            db.append_quota_success(&QuotaSnapshot {
                snapshot_id: format!("cold-{hour}"),
                provider_id: "cold-start-sub".to_string(),
                fetched_at,
                five_hour_utilization_percent: Some(hour.to_string()),
                five_hour_resets_at: Some(reset.clone()),
                seven_day_utilization_percent: None,
                seven_day_resets_at: None,
                manual_resets_remaining: None,
                raw_payload: json!({}),
                created_at: fetched_at,
            })
            .unwrap();
        }
        let snapshot = db.latest_quota_snapshot("cold-start-sub").unwrap().unwrap();
        let [actual, _] = classify_subscription_windows(
            &db,
            "cold-start-sub",
            Some(&snapshot),
            now,
            SubscriptionThresholds::default(),
        )
        .unwrap();
        let expected = classify_subscription_window(
            Some("72"),
            PaceInput {
                now_timestamp: now,
                reset_timestamp: Some(reset_timestamp),
                window_length_seconds: Some(5 * 3_600),
                measured_rate_per_second: Some(
                    Decimal::ONE.checked_div(Decimal::from(3_600_i64)).unwrap(),
                ),
                ..PaceInput::default()
            },
            SubscriptionThresholds::default(),
        );

        assert_eq!(actual.status, expected.status);
        assert_eq!(actual.headroom_ratio, expected.headroom_ratio);
        assert_eq!(actual.projected_exhaust_at, expected.projected_exhaust_at);
        assert_eq!(actual.rhythm_adjustment, None);
    }
}
