//! BE-16 / ADR v7.19.1: daily trends, one source per query.
//! Hourly buckets belong to the local date of their start (approximate for
//! fractional-hour offsets). Historical zero costs do not reveal parseability.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params_from_iter, types::Value};
use rust_decimal::{prelude::ToPrimitive, Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::policy::{LicenseBuild, LicenseStatus, LicenseTier};
use crate::store::{lock_conn, Database};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrendGroupBy {
    Provider,
    Project,
    Client,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrendFilters {
    pub provider_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageTrendRequest {
    pub tz: String,
    pub from: String,
    pub to: String,
    pub granularity: String,
    pub group_by: Option<TrendGroupBy>,
    pub filters: Option<TrendFilters>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTrendPoint {
    pub date_local: String,
    pub requests: i64,
    pub tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_micro_usd: Option<i64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTrendSeries {
    pub group_key: String,
    pub group_label: String,
    pub points: Vec<UsageTrendPoint>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTrendResponse {
    pub series: Vec<UsageTrendSeries>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clamped_from: Option<String>,
}

struct QueryRange {
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
    clamped_from: Option<String>,
}

fn parse_date(value: &str, field: &str) -> Result<NaiveDate, AppError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|date| value.len() == 10 && date.format("%Y-%m-%d").to_string() == value)
        .ok_or_else(|| AppError::invalid_argument("Expected YYYY-MM-DD.", field))
}

pub(crate) fn usage_query_is_limited(license: &LicenseStatus) -> bool {
    license.build == LicenseBuild::Community || license.tier == LicenseTier::Free
}

pub(crate) fn require_usage_split(license: &LicenseStatus) -> Result<(), AppError> {
    if usage_query_is_limited(license) {
        return Err(AppError::pro_required("Grouped usage requires Pro."));
    }
    Ok(())
}

/// Shared daily range policy. Legacy timestamp adapters also pass through here;
/// heatmaps, today-hourly views, and event/request-log lists remain exempt.
fn clamp_usage_query(
    request: &UsageTrendRequest,
    license: &LicenseStatus,
    now: DateTime<Utc>,
) -> Result<QueryRange, AppError> {
    if request.granularity != "day" {
        return Err(AppError::invalid_argument(
            "Only day is supported.",
            "granularity",
        ));
    }
    let tz: Tz = request
        .tz
        .parse()
        .map_err(|_| AppError::invalid_argument("Expected an IANA timezone.", "tz"))?;
    let from = parse_date(&request.from, "from")?;
    let to = parse_date(&request.to, "to")?;
    if from > to {
        return Err(AppError::invalid_argument(
            "from must not follow to.",
            "from",
        ));
    }
    let limited = usage_query_is_limited(license);
    if limited && request.group_by.is_some() {
        return Err(AppError::pro_required("Grouped usage trends require Pro."));
    }
    let today = now.with_timezone(&tz).date_naive();
    let lower = today
        .checked_sub_days(Days::new(29))
        .ok_or_else(|| AppError::internal("Could not determine the usage window."))?;
    let (effective_from, effective_to) = if limited {
        (from.max(lower), to.min(today))
    } else {
        (from, to)
    };
    Ok(QueryRange {
        tz,
        from: effective_from,
        to: effective_to,
        // ADR v7.19.3 option (b): effective lower date, only for a start clamp.
        clamped_from: (effective_from != from).then(|| effective_from.to_string()),
    })
}

/// ADR v7.19.2 adapter for legacy local-time timestamp contracts. Keep partial
/// days and the caller's inclusive/half-open endpoint convention unchanged.
pub(crate) struct LegacyUsageRange {
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub clamped_from: Option<String>,
    pub empty: bool,
}

pub(crate) fn clamp_legacy_usage_query<T: TimeZone>(
    start: Option<i64>,
    end: Option<i64>,
    inclusive_end: bool,
    license: &LicenseStatus,
    now: DateTime<T>,
) -> Result<LegacyUsageRange, AppError> {
    if !usage_query_is_limited(license) {
        return Ok(LegacyUsageRange {
            start,
            end,
            clamped_from: None,
            empty: false,
        });
    }
    let tz = now.timezone();
    let today = now.date_naive();
    // UTC here carries local calendar dates only, not actual event instants.
    // This reuses BE-16's policy without requiring a new timezone argument on
    // legacy commands, whose existing contract uses the machine's local zone.
    let request = UsageTrendRequest {
        tz: "UTC".into(),
        from: "0001-01-01".into(),
        to: today.to_string(),
        granularity: "day".into(),
        group_by: None,
        filters: None,
    };
    let range = clamp_usage_query(&request, license, now.naive_local().and_utc())?;
    let midnight = |day: NaiveDate| {
        let start = day.and_hms_opt(0, 0, 0).unwrap();
        // Some zones advance clocks at midnight (or skip a whole date).
        // Usually the first candidate succeeds; in a gap find the first real
        // instant rather than rejecting an otherwise valid calendar window.
        (0..=86_400)
            .find_map(|seconds| {
                tz.from_local_datetime(&(start + chrono::Duration::seconds(seconds)))
                    .earliest()
            })
            .map(|date| date.timestamp())
            .ok_or_else(|| AppError::internal("Could not determine the local day boundary."))
    };
    let lower = range.from;
    let lower_ts = midnight(lower)?;
    let upper_ts = midnight(
        today
            .succ_opt()
            .ok_or_else(|| AppError::internal("Invalid usage day."))?,
    )? - i64::from(inclusive_end);
    let effective_start = start.map_or(lower_ts, |value| value.max(lower_ts));
    let effective_end = end.map_or(upper_ts, |value| value.min(upper_ts));
    Ok(LegacyUsageRange {
        start: Some(effective_start),
        end: Some(effective_end),
        // Explicit BE-16b refinement: effective lower date, not original from.
        clamped_from: (start.is_none() || start.is_some_and(|value| value < lower_ts))
            .then(|| lower.to_string()),
        empty: if inclusive_end {
            effective_start > effective_end
        } else {
            effective_start >= effective_end
        },
    })
}

#[derive(Default)]
struct DayTotals {
    requests: i64,
    tokens: i64,
    cost: Option<i64>,
}

#[derive(Default)]
struct SeriesTotals {
    label: String,
    days: BTreeMap<NaiveDate, DayTotals>,
}

fn add(left: i64, right: i64) -> Result<i64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::internal("Usage total is out of range."))
}

fn raw_cost(value: Option<String>) -> Result<Option<i64>, AppError> {
    let Some(cost) = value.and_then(|text| Decimal::from_str(text.trim()).ok()) else {
        return Ok(None);
    };
    cost.checked_mul(Decimal::from(1_000_000))
        .and_then(|value| {
            value
                .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
                .to_i64()
        })
        .map(Some)
        .ok_or_else(|| AppError::internal("Usage cost is out of range."))
}

/// Value input deliberately converts malformed filters/enums to the coded
/// INVALID_ARGUMENT contract rather than a Tauri deserialization string.
pub fn query_usage_trend_at(
    db: &Database,
    input: serde_json::Value,
    license: &LicenseStatus,
    now: DateTime<Utc>,
) -> Result<UsageTrendResponse, AppError> {
    let request: UsageTrendRequest = serde_json::from_value(input)
        .map_err(|_| AppError::invalid_argument("Invalid usage trend request.", "request"))?;
    let range = clamp_usage_query(&request, license, now)?;
    query(db, &request, range).map_err(|error| match error {
        AppError::Coded { .. } => error,
        _ => AppError::internal("Could not query usage trends."),
    })
}

fn query(
    db: &Database,
    request: &UsageTrendRequest,
    range: QueryRange,
) -> Result<UsageTrendResponse, AppError> {
    let mut groups = BTreeMap::<String, SeriesTotals>::new();
    // Keep the empty/all or unattributed series even when there are no rows.
    groups.insert(
        if request.group_by.is_none() {
            "all"
        } else {
            "unattributed"
        }
        .into(),
        SeriesTotals::default(),
    );
    if range.from <= range.to {
        read_rows(db, request, &range, &mut groups)?;
    }
    let mut series = Vec::with_capacity(groups.len());
    for (group_key, totals) in groups {
        let mut points = Vec::new();
        let mut date = range.from;
        while date <= range.to {
            let day = totals.days.get(&date);
            points.push(UsageTrendPoint {
                date_local: date.format("%Y-%m-%d").to_string(),
                requests: day.map_or(0, |day| day.requests),
                tokens: day.map_or(0, |day| day.tokens),
                cost_micro_usd: day.map_or(Some(0), |day| day.cost),
            });
            if date == range.to {
                break;
            }
            date = date
                .succ_opt()
                .ok_or_else(|| AppError::internal("Usage date is out of range."))?;
        }
        series.push(UsageTrendSeries {
            group_key,
            group_label: totals.label,
            points,
        });
    }
    Ok(UsageTrendResponse {
        series,
        clamped_from: range.clamped_from,
    })
}

fn read_rows(
    db: &Database,
    request: &UsageTrendRequest,
    range: &QueryRange,
    groups: &mut BTreeMap<String, SeriesTotals>,
) -> Result<(), AppError> {
    let raw = matches!(
        request.group_by,
        Some(TrendGroupBy::Project | TrendGroupBy::Client)
    );
    let (table, timestamp, dimension, requests, cost) = if raw {
        (
            "proxy_request_logs",
            "created_at",
            if request.group_by == Some(TrendGroupBy::Project) {
                "project_alias"
            } else {
                "client_tag_id"
            },
            "1",
            "total_cost_usd",
        )
    } else {
        (
            "usage_hourly_rollups",
            "hour_utc",
            "provider_id",
            "request_count",
            "cost_micro_usd",
        )
    };
    // A broad UTC envelope avoids assuming that local midnight exists or is
    // unique. Exact inclusion is determined from each instant's IANA date.
    let start = range
        .from
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
        - 86_400;
    let end = range.to.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp() + 2 * 86_400;
    let mut parameters = vec![Value::Integer(start), Value::Integer(end)];
    let mut filter = String::new();
    if let Some(ids) = request
        .filters
        .as_ref()
        .and_then(|filters| filters.provider_ids.as_ref())
    {
        let ids: BTreeSet<_> = ids.iter().collect();
        if ids.is_empty() {
            return Ok(());
        }
        filter = format!(" AND r.provider_id IN ({})", vec!["?"; ids.len()].join(","));
        parameters.extend(ids.into_iter().map(|id| Value::Text(id.clone())));
    }
    let label = if request.group_by == Some(TrendGroupBy::Provider) {
        "COALESCE(
            NULLIF((SELECT name FROM usage_providers WHERE id = r.provider_id), ''),
            NULLIF((SELECT name FROM usage_providers WHERE legacy_provider_id = r.provider_id AND legacy_app_type = r.app_type), ''),
            NULLIF((SELECT name FROM providers WHERE id = r.provider_id AND app_type = r.app_type), ''),
            r.provider_id)"
    } else {
        // No client-name registry exists in the public schema: use the
        // architect-approved key fallback. Project keys already are aliases.
        dimension
    };
    let sql = format!(
        "SELECT r.{timestamp}, r.{dimension}, {label}, {requests},
                r.input_tokens, r.output_tokens, r.cache_read_tokens,
                r.cache_creation_tokens, r.{cost}
         FROM {table} r WHERE r.{timestamp} >= ? AND r.{timestamp} < ?{filter}
         ORDER BY r.{timestamp}, r.{dimension}, r.app_type"
    );
    let conn = lock_conn!(db.conn);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    while let Some(row) = rows.next()? {
        let instant = DateTime::from_timestamp(row.get(0)?, 0)
            .ok_or_else(|| AppError::internal("Stored usage timestamp is out of range."))?;
        let date = instant.with_timezone(&range.tz).date_naive();
        if date < range.from || date > range.to {
            continue;
        }
        let dimension: Option<String> = row.get(1)?;
        let key = if request.group_by.is_none() {
            "all".to_string()
        } else {
            dimension
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| "unattributed".into())
        };
        let group = groups.entry(key.clone()).or_default();
        if request.group_by.is_some() && key != "unattributed" {
            let label: Option<String> = row.get(2)?;
            group.label = label.filter(|label| !label.is_empty()).unwrap_or(key);
        }
        let day = group.days.entry(date).or_default();
        day.requests = add(day.requests, row.get(3)?)?;
        // Total observed tokens includes all four stored token categories.
        for index in 4..=7 {
            day.tokens = add(day.tokens, row.get(index)?)?;
        }
        let cost = if raw {
            raw_cost(row.get(8)?)?
        } else {
            row.get::<_, Option<i64>>(8)?
        };
        if let Some(cost) = cost {
            day.cost = Some(add(day.cost.unwrap_or(0), cost)?);
        }
    }
    Ok(())
}
