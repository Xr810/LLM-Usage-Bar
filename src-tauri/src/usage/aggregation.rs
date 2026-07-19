use crate::database::Database;
use crate::error::AppError;
use crate::usage::domain::{CostSourceCounts, UsageTrendBucketView, UsageTrendGranularity};
use chrono::{Duration, Local, NaiveDate, TimeZone};
use rusqlite::params;
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRangeAggregate {
    pub event_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: Option<String>,
    pub cost_source_counts: CostSourceCounts,
}

pub fn aggregate_enabled_provider_trend(
    db: &Database,
    start_at: i64,
    end_at: i64,
) -> Result<(UsageTrendGranularity, Vec<UsageTrendBucketView>), AppError> {
    if start_at >= end_at {
        return Err(AppError::Message(
            "start_at must be before end_at".to_string(),
        ));
    }

    let granularity = if end_at - start_at <= 24 * 60 * 60 {
        UsageTrendGranularity::Hour
    } else {
        UsageTrendGranularity::Day
    };
    let mut buckets = empty_trend_buckets(start_at, end_at, granularity)?;
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let mut statement = conn.prepare(
        "SELECT event.occurred_at, event.input_tokens, event.output_tokens,
                event.cache_read_tokens, event.cache_creation_tokens
         FROM usage_events AS event
         JOIN usage_providers AS provider ON provider.id = event.provider_id
         WHERE provider.enabled = 1
           AND event.occurred_at >= ?1 AND event.occurred_at < ?2
           AND NOT EXISTS (
               SELECT 1
               FROM usage_event_links AS link
               JOIN usage_events AS canonical
                 ON canonical.event_id = link.canonical_event_id
               JOIN usage_events AS duplicate
                 ON duplicate.event_id = link.duplicate_event_id
               WHERE duplicate.event_id = event.event_id
                 AND canonical.source = 'proxy'
                 AND duplicate.source = 'session_log'
                 AND canonical.provider_id = duplicate.provider_id
           )
         ORDER BY event.occurred_at, event.event_id",
    )?;
    let rows = statement.query_map(params![start_at, end_at], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    for row in rows {
        let (occurred_at, input, output, cache_read, cache_creation) = row?;
        let index = trend_bucket_index(start_at, occurred_at, granularity)?;
        let bucket = buckets.get_mut(index).ok_or_else(|| {
            AppError::Database("usage event fell outside trend buckets".to_string())
        })?;
        bucket.event_count = checked_sum(bucket.event_count, 1)?;
        let event_tokens = [
            (input, "input_tokens"),
            (output, "output_tokens"),
            (cache_read, "cache_read_tokens"),
            (cache_creation, "cache_creation_tokens"),
        ]
        .into_iter()
        .try_fold(0_u64, |total, (value, field)| {
            checked_sum(total, non_negative(value, field)?)
        })?;
        bucket.total_tokens = checked_sum(bucket.total_tokens, event_tokens)?;
    }

    Ok((granularity, buckets))
}

fn empty_trend_buckets(
    start_at: i64,
    end_at: i64,
    granularity: UsageTrendGranularity,
) -> Result<Vec<UsageTrendBucketView>, AppError> {
    match granularity {
        UsageTrendGranularity::Hour => {
            let count = (end_at - start_at + 60 * 60 - 1) / (60 * 60);
            (0..count)
                .map(|index| {
                    let bucket_start = start_at + index * 60 * 60;
                    Ok(UsageTrendBucketView {
                        start_at: bucket_start,
                        end_at: (bucket_start + 60 * 60).min(end_at),
                        event_count: 0,
                        total_tokens: 0,
                    })
                })
                .collect()
        }
        UsageTrendGranularity::Day => {
            let start_date = local_date(start_at)?;
            let last_date = local_date(end_at - 1)?;
            let count = last_date.signed_duration_since(start_date).num_days() + 1;
            (0..count)
                .map(|index| {
                    let date = start_date
                        .checked_add_signed(Duration::days(index))
                        .ok_or_else(|| AppError::Database("trend date overflow".to_string()))?;
                    let next_date = date
                        .checked_add_signed(Duration::days(1))
                        .ok_or_else(|| AppError::Database("trend date overflow".to_string()))?;
                    Ok(UsageTrendBucketView {
                        start_at: local_midnight_timestamp(date)?.max(start_at),
                        end_at: local_midnight_timestamp(next_date)?.min(end_at),
                        event_count: 0,
                        total_tokens: 0,
                    })
                })
                .collect()
        }
    }
}

fn trend_bucket_index(
    start_at: i64,
    occurred_at: i64,
    granularity: UsageTrendGranularity,
) -> Result<usize, AppError> {
    let index = match granularity {
        UsageTrendGranularity::Hour => (occurred_at - start_at) / (60 * 60),
        UsageTrendGranularity::Day => local_date(occurred_at)?
            .signed_duration_since(local_date(start_at)?)
            .num_days(),
    };
    usize::try_from(index)
        .map_err(|_| AppError::Database("invalid usage trend bucket index".to_string()))
}

fn local_date(timestamp: i64) -> Result<NaiveDate, AppError> {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.date_naive())
        .ok_or_else(|| AppError::Database("invalid usage trend timestamp".to_string()))
}

fn local_midnight_timestamp(date: NaiveDate) -> Result<i64, AppError> {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::Database("invalid usage trend date".to_string()))?;
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|value| value.timestamp())
        .ok_or_else(|| AppError::Database("invalid local midnight".to_string()))
}

pub fn aggregate_provider_range(
    db: &Database,
    agent_module_id: &str,
    provider_id: &str,
    product_group_id: &str,
    start_at: i64,
    end_at: i64,
) -> Result<ProviderRangeAggregate, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let mut statement = conn.prepare(
        "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_cost_usd, cost_source
         FROM usage_events AS event
         WHERE event.provider_id = ?1
           AND event.product_group_id = ?2
           AND event.occurred_at >= ?3 AND event.occurred_at < ?4
           AND event.agent_module_id = ?5
           AND NOT EXISTS (
               SELECT 1
               FROM usage_event_links AS link
               JOIN usage_events AS canonical
                 ON canonical.event_id = link.canonical_event_id
               JOIN usage_events AS duplicate
                 ON duplicate.event_id = link.duplicate_event_id
               WHERE duplicate.event_id = event.event_id
                 AND canonical.source = 'proxy'
                 AND duplicate.source = 'session_log'
                 AND canonical.provider_id = duplicate.provider_id
                 AND canonical.agent_module_id IS NOT NULL
                 AND canonical.agent_module_id = duplicate.agent_module_id
           )
         ORDER BY event.occurred_at, event.event_id",
    )?;
    let rows = statement.query_map(
        params![
            provider_id,
            product_group_id,
            start_at,
            end_at,
            agent_module_id
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        },
    )?;

    let mut event_count = 0_u64;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut cache_read_tokens = 0_u64;
    let mut cache_creation_tokens = 0_u64;
    let mut total_cost = Decimal::ZERO;
    let mut has_cost = false;
    let mut cost_source_counts = CostSourceCounts::default();
    for row in rows {
        let (input, output, cache_read, cache_creation, cost, source) = row?;
        event_count = checked_sum(event_count, 1)?;
        input_tokens = checked_sum(input_tokens, non_negative(input, "input_tokens")?)?;
        output_tokens = checked_sum(output_tokens, non_negative(output, "output_tokens")?)?;
        cache_read_tokens = checked_sum(
            cache_read_tokens,
            non_negative(cache_read, "cache_read_tokens")?,
        )?;
        cache_creation_tokens = checked_sum(
            cache_creation_tokens,
            non_negative(cache_creation, "cache_creation_tokens")?,
        )?;
        if let Some(cost) = cost {
            total_cost = checked_cost_sum(total_cost, parse_decimal(&cost)?)?;
            has_cost = true;
        }
        match source.as_str() {
            "upstream" => {
                cost_source_counts.upstream = checked_sum(cost_source_counts.upstream, 1)?
            }
            "estimated" => {
                cost_source_counts.estimated = checked_sum(cost_source_counts.estimated, 1)?
            }
            "unavailable" => {
                cost_source_counts.unavailable = checked_sum(cost_source_counts.unavailable, 1)?
            }
            _ => {
                return Err(AppError::Database(format!(
                    "invalid usage cost source: {source}"
                )))
            }
        }
    }

    Ok(ProviderRangeAggregate {
        event_count,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_cost_usd: has_cost.then(|| total_cost.normalize().to_string()),
        cost_source_counts,
    })
}

/// Aggregate one Provider account across every historical Agent and product
/// group. Provider identity is the accounting boundary; explicit cross-source
/// duplicate links remain authoritative even when legacy Agent metadata differs.
pub fn aggregate_provider_account_range(
    db: &Database,
    provider_id: &str,
    start_at: i64,
    end_at: i64,
) -> Result<ProviderRangeAggregate, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let mut statement = conn.prepare(
        "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_cost_usd, cost_source
         FROM usage_events AS event
         WHERE event.provider_id = ?1
           AND event.occurred_at >= ?2 AND event.occurred_at < ?3
           AND NOT EXISTS (
               SELECT 1
               FROM usage_event_links AS link
               JOIN usage_events AS canonical
                 ON canonical.event_id = link.canonical_event_id
               JOIN usage_events AS duplicate
                 ON duplicate.event_id = link.duplicate_event_id
               WHERE duplicate.event_id = event.event_id
                 AND canonical.source = 'proxy'
                 AND duplicate.source = 'session_log'
                 AND canonical.provider_id = duplicate.provider_id
           )
         ORDER BY event.occurred_at, event.event_id",
    )?;
    let rows = statement.query_map(params![provider_id, start_at, end_at], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    aggregate_rows(rows)
}

fn aggregate_rows(
    rows: impl Iterator<Item = rusqlite::Result<(i64, i64, i64, i64, Option<String>, String)>>,
) -> Result<ProviderRangeAggregate, AppError> {
    let mut event_count = 0_u64;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut cache_read_tokens = 0_u64;
    let mut cache_creation_tokens = 0_u64;
    let mut total_cost = Decimal::ZERO;
    let mut has_cost = false;
    let mut cost_source_counts = CostSourceCounts::default();
    for row in rows {
        let (input, output, cache_read, cache_creation, cost, source) = row?;
        event_count = checked_sum(event_count, 1)?;
        input_tokens = checked_sum(input_tokens, non_negative(input, "input_tokens")?)?;
        output_tokens = checked_sum(output_tokens, non_negative(output, "output_tokens")?)?;
        cache_read_tokens = checked_sum(
            cache_read_tokens,
            non_negative(cache_read, "cache_read_tokens")?,
        )?;
        cache_creation_tokens = checked_sum(
            cache_creation_tokens,
            non_negative(cache_creation, "cache_creation_tokens")?,
        )?;
        if let Some(cost) = cost {
            total_cost = checked_cost_sum(total_cost, parse_decimal(&cost)?)?;
            has_cost = true;
        }
        match source.as_str() {
            "upstream" => {
                cost_source_counts.upstream = checked_sum(cost_source_counts.upstream, 1)?
            }
            "estimated" => {
                cost_source_counts.estimated = checked_sum(cost_source_counts.estimated, 1)?
            }
            "unavailable" => {
                cost_source_counts.unavailable = checked_sum(cost_source_counts.unavailable, 1)?
            }
            _ => {
                return Err(AppError::Database(format!(
                    "invalid usage cost source: {source}"
                )))
            }
        }
    }

    Ok(ProviderRangeAggregate {
        event_count,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        total_cost_usd: has_cost.then(|| total_cost.normalize().to_string()),
        cost_source_counts,
    })
}

fn non_negative(value: i64, field: &str) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| AppError::Database(format!("negative usage value in {field}")))
}

fn checked_sum(left: u64, right: u64) -> Result<u64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Database("usage aggregate overflow".to_string()))
}

fn parse_decimal(value: &str) -> Result<Decimal, AppError> {
    Decimal::from_str(value)
        .map_err(|error| AppError::Database(format!("invalid usage cost {value}: {error}")))
}

fn checked_cost_sum(left: Decimal, right: Decimal) -> Result<Decimal, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Database("usage cost aggregate overflow".to_string()))
}

#[cfg(test)]
mod tests {
    use super::aggregate_provider_range;
    use crate::database::Database;
    use crate::usage::domain::{
        BillingKind, CostSource, TokenSource, UsageEvent, UsageEventLink, UsageProviderInput,
    };

    fn provider() -> UsageProviderInput {
        UsageProviderInput {
            id: "metered".to_string(),
            name: "Metered".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "openai".to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn event(
        event_id: &str,
        source: TokenSource,
        agent_module_id: &str,
        provider_id: &str,
        product_group_id: &str,
        occurred_at: i64,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
        total_cost_usd: Option<&str>,
        cost_source: CostSource,
    ) -> UsageEvent {
        UsageEvent {
            event_id: event_id.to_string(),
            source,
            provider_id: provider_id.to_string(),
            agent_module_id: Some(agent_module_id.to_string()),
            product_group_id: product_group_id.to_string(),
            occurred_at,
            model: "model".to_string(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: None,
            output_cost_usd: None,
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: total_cost_usd.map(str::to_string),
            cost_source,
            legacy_request_id: None,
            created_at: occurred_at,
        }
    }

    #[test]
    fn aggregates_exact_costs_tokens_sources_and_excludes_linked_duplicates_and_end_bound() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider()).unwrap();

        let canonical = event(
            "canonical",
            TokenSource::Proxy,
            "codex",
            "metered",
            "openai",
            1_000,
            100,
            50,
            10,
            5,
            Some("1.23456789"),
            CostSource::Upstream,
        );
        let duplicate = event(
            "duplicate",
            TokenSource::SessionLog,
            "codex",
            "metered",
            "openai",
            1_001,
            100,
            50,
            10,
            5,
            Some("9"),
            CostSource::Estimated,
        );
        let estimated = event(
            "estimated",
            TokenSource::SessionLog,
            "codex",
            "metered",
            "openai",
            1_500,
            50,
            25,
            20,
            10,
            Some("0"),
            CostSource::Estimated,
        );
        let unavailable = event(
            "unavailable",
            TokenSource::Proxy,
            "codex",
            "metered",
            "openai",
            1_999,
            0,
            0,
            30,
            15,
            None,
            CostSource::Unavailable,
        );
        let end_bound = event(
            "end-bound",
            TokenSource::Proxy,
            "codex",
            "metered",
            "openai",
            2_000,
            9_999,
            9_999,
            9_999,
            9_999,
            Some("9999"),
            CostSource::Upstream,
        );
        let other_agent = event(
            "other-agent",
            TokenSource::Proxy,
            "claude-code",
            "metered",
            "openai",
            1_500,
            9_999,
            9_999,
            9_999,
            9_999,
            Some("9999"),
            CostSource::Upstream,
        );

        for usage_event in [
            &canonical,
            &duplicate,
            &estimated,
            &unavailable,
            &end_bound,
            &other_agent,
        ] {
            db.insert_usage_event(usage_event).unwrap();
        }
        db.insert_usage_event_link(&UsageEventLink {
            canonical_event_id: canonical.event_id.clone(),
            duplicate_event_id: duplicate.event_id.clone(),
            link_kind: "request_id".to_string(),
            link_value: "shared-request".to_string(),
            created_at: 1_001,
        })
        .unwrap();

        let result =
            aggregate_provider_range(&db, "codex", "metered", "openai", 1_000, 2_000).unwrap();

        assert_eq!(result.event_count, 3);
        assert_eq!(result.input_tokens, 150);
        assert_eq!(result.output_tokens, 75);
        assert_eq!(result.cache_read_tokens, 60);
        assert_eq!(result.cache_creation_tokens, 30);
        assert_eq!(result.total_cost_usd.as_deref(), Some("1.23456789"));
        assert_eq!(result.cost_source_counts.upstream, 1);
        assert_eq!(result.cost_source_counts.estimated, 1);
        assert_eq!(result.cost_source_counts.unavailable, 1);
    }
}
