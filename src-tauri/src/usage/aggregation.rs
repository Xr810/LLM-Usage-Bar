use crate::error::AppError;
use crate::model::{
    AgentProviderUsageRow, AgentUsageBreakdownView, AgentUsageRow, BillingKind, CostSourceCounts,
    ModelProductGroupView, ModelTotalsRow, ModelUsageDashboardView, ModelUsageRow,
    UsageTrendBucketView, UsageTrendGranularity,
};
use crate::store::Database;
use chrono::{Duration, Local, NaiveDate, TimeZone};
use rusqlite::params;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

const NOT_A_LINKED_SESSION_DUPLICATE: &str = "NOT EXISTS (
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
           )";

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
    aggregate_provider_trend(db, None, start_at, end_at)
}

pub fn aggregate_provider_account_trend(
    db: &Database,
    provider_id: &str,
    start_at: i64,
    end_at: i64,
) -> Result<(UsageTrendGranularity, Vec<UsageTrendBucketView>), AppError> {
    aggregate_provider_trend(db, Some(provider_id), start_at, end_at)
}

pub fn most_used_provider_model(
    db: &Database,
    provider_id: &str,
    start_at: i64,
    end_at: i64,
) -> Result<Option<String>, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let mut statement = conn.prepare(
        "SELECT event.model
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
         GROUP BY event.model
         ORDER BY COUNT(*) DESC, event.model ASC
         LIMIT 1",
    )?;
    let mut rows = statement.query(params![provider_id, start_at, end_at])?;
    Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
}

fn aggregate_provider_trend(
    db: &Database,
    provider_id: Option<&str>,
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
                event.cache_read_tokens, event.cache_creation_tokens,
                event.total_cost_usd, event.cost_source
         FROM usage_events AS event
         JOIN usage_providers AS provider ON provider.id = event.provider_id
         WHERE provider.enabled = 1
           AND (?1 IS NULL OR event.provider_id = ?1)
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
            row.get::<_, i64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    for row in rows {
        let (occurred_at, input, output, cache_read, cache_creation, cost, cost_source) = row?;
        let index = trend_bucket_index(start_at, occurred_at, granularity)?;
        let bucket = buckets.get_mut(index).ok_or_else(|| {
            AppError::Database("usage event fell outside trend buckets".to_string())
        })?;
        bucket.event_count = checked_sum(bucket.event_count, 1)?;
        let input = non_negative(input, "input_tokens")?;
        let output = non_negative(output, "output_tokens")?;
        let cache_read = non_negative(cache_read, "cache_read_tokens")?;
        let cache_creation = non_negative(cache_creation, "cache_creation_tokens")?;
        bucket.input_tokens = checked_sum(bucket.input_tokens, input)?;
        bucket.output_tokens = checked_sum(bucket.output_tokens, output)?;
        bucket.cache_read_tokens = checked_sum(bucket.cache_read_tokens, cache_read)?;
        bucket.cache_creation_tokens = checked_sum(bucket.cache_creation_tokens, cache_creation)?;
        let event_tokens = [input, output, cache_read, cache_creation]
            .into_iter()
            .try_fold(0_u64, checked_sum)?;
        bucket.total_tokens = checked_sum(bucket.total_tokens, event_tokens)?;
        if let Some(cost) = cost {
            let total = bucket
                .total_cost_usd
                .as_deref()
                .map(parse_decimal)
                .transpose()?
                .unwrap_or(Decimal::ZERO);
            bucket.total_cost_usd = Some(
                checked_cost_sum(total, parse_decimal(&cost)?)?
                    .normalize()
                    .to_string(),
            );
        }
        match cost_source.as_str() {
            "upstream" => {
                bucket.cost_source_counts.upstream =
                    checked_sum(bucket.cost_source_counts.upstream, 1)?
            }
            "estimated" => {
                bucket.cost_source_counts.estimated =
                    checked_sum(bucket.cost_source_counts.estimated, 1)?
            }
            "unavailable" => {
                bucket.cost_source_counts.unavailable =
                    checked_sum(bucket.cost_source_counts.unavailable, 1)?
            }
            _ => {
                return Err(AppError::Database(format!(
                    "invalid usage cost source: {cost_source}"
                )))
            }
        }
    }

    Ok((granularity, buckets))
}

#[derive(Debug, Clone, Copy)]
enum AggregateCostSource {
    Upstream,
    Estimated,
    Unavailable,
}

#[derive(Debug)]
struct UsageSample {
    occurred_at: i64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_tokens: u64,
    total_cost: Option<Decimal>,
    cost_source: AggregateCostSource,
}

impl UsageSample {
    #[allow(clippy::too_many_arguments)]
    fn new(
        occurred_at: i64,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        total_cost_usd: Option<String>,
        cost_source: String,
    ) -> Result<Self, AppError> {
        let input_tokens = non_negative(input_tokens, "input_tokens")?;
        let output_tokens = non_negative(output_tokens, "output_tokens")?;
        let cache_read_tokens = non_negative(cache_read_tokens, "cache_read_tokens")?;
        let cache_creation_tokens = non_negative(cache_creation_tokens, "cache_creation_tokens")?;
        let total_tokens = [
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        ]
        .into_iter()
        .try_fold(0_u64, checked_sum)?;
        let total_cost = total_cost_usd.as_deref().map(parse_decimal).transpose()?;
        let cost_source = match cost_source.as_str() {
            "upstream" => AggregateCostSource::Upstream,
            "estimated" => AggregateCostSource::Estimated,
            "unavailable" => AggregateCostSource::Unavailable,
            _ => {
                return Err(AppError::Database(format!(
                    "invalid usage cost source: {cost_source}"
                )))
            }
        };
        Ok(Self {
            occurred_at,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            total_tokens,
            total_cost,
            cost_source,
        })
    }
}

#[derive(Debug, Default)]
struct UsageAccumulator {
    event_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_tokens: u64,
    total_cost: Option<Decimal>,
    cost_source_counts: CostSourceCounts,
    first_occurred_at: Option<i64>,
    last_occurred_at: Option<i64>,
}

impl UsageAccumulator {
    fn add(&mut self, sample: &UsageSample) -> Result<(), AppError> {
        self.event_count = checked_sum(self.event_count, 1)?;
        self.input_tokens = checked_sum(self.input_tokens, sample.input_tokens)?;
        self.output_tokens = checked_sum(self.output_tokens, sample.output_tokens)?;
        self.cache_read_tokens = checked_sum(self.cache_read_tokens, sample.cache_read_tokens)?;
        self.cache_creation_tokens =
            checked_sum(self.cache_creation_tokens, sample.cache_creation_tokens)?;
        self.total_tokens = checked_sum(self.total_tokens, sample.total_tokens)?;
        if let Some(cost) = sample.total_cost {
            let current = self.total_cost.take().unwrap_or(Decimal::ZERO);
            self.total_cost = Some(checked_cost_sum(current, cost)?);
        }
        match sample.cost_source {
            AggregateCostSource::Upstream => {
                self.cost_source_counts.upstream =
                    checked_sum(self.cost_source_counts.upstream, 1)?;
            }
            AggregateCostSource::Estimated => {
                self.cost_source_counts.estimated =
                    checked_sum(self.cost_source_counts.estimated, 1)?;
            }
            AggregateCostSource::Unavailable => {
                self.cost_source_counts.unavailable =
                    checked_sum(self.cost_source_counts.unavailable, 1)?;
            }
        }
        self.first_occurred_at = Some(
            self.first_occurred_at
                .map_or(sample.occurred_at, |current| {
                    current.min(sample.occurred_at)
                }),
        );
        self.last_occurred_at = Some(self.last_occurred_at.map_or(sample.occurred_at, |current| {
            current.max(sample.occurred_at)
        }));
        Ok(())
    }

    fn total_cost_usd(&self) -> Option<String> {
        self.total_cost.map(|cost| cost.normalize().to_string())
    }

    fn timestamps(&self) -> Result<(i64, i64), AppError> {
        self.first_occurred_at
            .zip(self.last_occurred_at)
            .ok_or_else(|| AppError::Database("empty usage aggregate".to_string()))
    }
}

#[derive(Debug)]
struct RawEnabledUsageEvent {
    agent_module_id: Option<String>,
    agent_name: Option<String>,
    agent_archived_at: Option<i64>,
    agent_visible: Option<bool>,
    provider_id: String,
    provider_name: String,
    product_group_id: String,
    billing_kind: String,
    model: String,
    occurred_at: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    total_cost_usd: Option<String>,
    cost_source: String,
}

#[derive(Debug)]
struct EnabledUsageEvent {
    agent_module_id: Option<String>,
    agent_name: Option<String>,
    agent_archived_at: Option<i64>,
    agent_visible: Option<bool>,
    provider_id: String,
    provider_name: String,
    product_group_id: String,
    billing_kind: BillingKind,
    model: String,
    sample: UsageSample,
}

fn parse_billing_kind(value: &str) -> Result<BillingKind, AppError> {
    match value {
        "subscription" => Ok(BillingKind::Subscription),
        "metered" => Ok(BillingKind::Metered),
        _ => Err(AppError::Database(format!(
            "invalid provider billing kind: {value}"
        ))),
    }
}

fn for_each_enabled_usage_event(
    db: &Database,
    start_at: i64,
    end_at: i64,
    mut visit: impl FnMut(EnabledUsageEvent) -> Result<(), AppError>,
) -> Result<(), AppError> {
    if start_at >= end_at {
        return Err(AppError::Message(
            "start_at must be before end_at".to_string(),
        ));
    }

    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let query = format!(
        "SELECT event.agent_module_id, agent.name, agent.archived_at, agent.visible,
                provider.id, provider.name, provider.product_group_id, provider.billing_kind,
                event.model, event.occurred_at, event.input_tokens, event.output_tokens,
                event.cache_read_tokens, event.cache_creation_tokens,
                event.total_cost_usd, event.cost_source
         FROM usage_events AS event
         JOIN usage_providers AS provider ON provider.id = event.provider_id
         LEFT JOIN agent_modules AS agent ON agent.id = event.agent_module_id
         WHERE provider.enabled = 1
           AND event.occurred_at >= ?1 AND event.occurred_at < ?2
           AND {NOT_A_LINKED_SESSION_DUPLICATE}
         ORDER BY event.occurred_at, event.event_id"
    );
    let mut statement = conn.prepare(&query)?;
    let rows = statement.query_map(params![start_at, end_at], |row| {
        Ok(RawEnabledUsageEvent {
            agent_module_id: row.get(0)?,
            agent_name: row.get(1)?,
            agent_archived_at: row.get(2)?,
            agent_visible: row.get(3)?,
            provider_id: row.get(4)?,
            provider_name: row.get(5)?,
            product_group_id: row.get(6)?,
            billing_kind: row.get(7)?,
            model: row.get(8)?,
            occurred_at: row.get(9)?,
            input_tokens: row.get(10)?,
            output_tokens: row.get(11)?,
            cache_read_tokens: row.get(12)?,
            cache_creation_tokens: row.get(13)?,
            total_cost_usd: row.get(14)?,
            cost_source: row.get(15)?,
        })
    })?;

    for row in rows {
        let row = row?;
        let billing_kind = parse_billing_kind(&row.billing_kind)?;
        let sample = UsageSample::new(
            row.occurred_at,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_creation_tokens,
            row.total_cost_usd,
            row.cost_source,
        )?;
        visit(EnabledUsageEvent {
            agent_module_id: row.agent_module_id,
            agent_name: row.agent_name,
            agent_archived_at: row.agent_archived_at,
            agent_visible: row.agent_visible,
            provider_id: row.provider_id,
            provider_name: row.provider_name,
            product_group_id: row.product_group_id,
            billing_kind,
            model: row.model,
            sample,
        })?;
    }
    Ok(())
}

fn normalized_model(model: &str) -> String {
    if model.trim().is_empty() {
        "unknown".to_string()
    } else {
        model.to_string()
    }
}

#[derive(Debug)]
struct ModelLeafAccumulator {
    provider_name: String,
    billing_kind: BillingKind,
    usage: UsageAccumulator,
}

#[derive(Debug, Default)]
struct ModelProductAccumulator {
    has_subscription: bool,
    has_metered: bool,
    providers: BTreeMap<String, String>,
    usage: UsageAccumulator,
    models: BTreeMap<(String, String), ModelLeafAccumulator>,
}

#[derive(Debug, Default)]
struct ModelTotalsAccumulator {
    provider_ids: BTreeSet<String>,
    usage: UsageAccumulator,
}

fn model_totals_row(
    model: String,
    accumulator: ModelTotalsAccumulator,
) -> Result<ModelTotalsRow, AppError> {
    let (first_occurred_at, last_occurred_at) = accumulator.usage.timestamps()?;
    Ok(ModelTotalsRow {
        model,
        provider_ids: accumulator.provider_ids.into_iter().collect(),
        event_count: accumulator.usage.event_count,
        input_tokens: accumulator.usage.input_tokens,
        output_tokens: accumulator.usage.output_tokens,
        cache_read_tokens: accumulator.usage.cache_read_tokens,
        cache_creation_tokens: accumulator.usage.cache_creation_tokens,
        total_tokens: accumulator.usage.total_tokens,
        total_cost_usd: accumulator.usage.total_cost_usd(),
        cost_source_counts: accumulator.usage.cost_source_counts.clone(),
        first_occurred_at,
        last_occurred_at,
    })
}

pub fn aggregate_model_usage(
    db: &Database,
    start_at: i64,
    end_at: i64,
) -> Result<ModelUsageDashboardView, AppError> {
    let mut dashboard_usage = UsageAccumulator::default();
    let mut product_accumulators = BTreeMap::<String, ModelProductAccumulator>::new();
    let mut model_accumulators = BTreeMap::<String, ModelTotalsAccumulator>::new();

    for_each_enabled_usage_event(db, start_at, end_at, |event| {
        let model = normalized_model(&event.model);
        dashboard_usage.add(&event.sample)?;

        let product = product_accumulators
            .entry(event.product_group_id.clone())
            .or_default();
        match event.billing_kind {
            BillingKind::Subscription => product.has_subscription = true,
            BillingKind::Metered => product.has_metered = true,
        }
        product
            .providers
            .insert(event.provider_id.clone(), event.provider_name.clone());
        product.usage.add(&event.sample)?;
        product
            .models
            .entry((event.provider_id.clone(), model.clone()))
            .or_insert_with(|| ModelLeafAccumulator {
                provider_name: event.provider_name.clone(),
                billing_kind: event.billing_kind,
                usage: UsageAccumulator::default(),
            })
            .usage
            .add(&event.sample)?;

        let model_total = model_accumulators.entry(model).or_default();
        model_total.provider_ids.insert(event.provider_id);
        model_total.usage.add(&event.sample)
    })?;

    let mut warnings = Vec::new();
    let mut product_groups = Vec::with_capacity(product_accumulators.len());
    for (product_group_id, accumulator) in product_accumulators {
        let billing_kind = match (accumulator.has_subscription, accumulator.has_metered) {
            (true, true) => {
                warnings.push(format!("mixed_billing_kind:{product_group_id}"));
                BillingKind::Metered
            }
            (true, false) => BillingKind::Subscription,
            _ => BillingKind::Metered,
        };
        let (provider_ids, provider_names): (Vec<_>, Vec<_>) =
            accumulator.providers.into_iter().unzip();
        let mut models = accumulator
            .models
            .into_iter()
            .map(|((provider_id, model), leaf)| {
                let (first_occurred_at, last_occurred_at) = leaf.usage.timestamps()?;
                Ok(ModelUsageRow {
                    model,
                    provider_id,
                    provider_name: leaf.provider_name,
                    product_group_id: product_group_id.clone(),
                    billing_kind: leaf.billing_kind,
                    event_count: leaf.usage.event_count,
                    input_tokens: leaf.usage.input_tokens,
                    output_tokens: leaf.usage.output_tokens,
                    cache_read_tokens: leaf.usage.cache_read_tokens,
                    cache_creation_tokens: leaf.usage.cache_creation_tokens,
                    total_tokens: leaf.usage.total_tokens,
                    total_cost_usd: leaf.usage.total_cost_usd(),
                    cost_source_counts: leaf.usage.cost_source_counts.clone(),
                    first_occurred_at,
                    last_occurred_at,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        models.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.model.cmp(&right.model))
                .then_with(|| left.provider_id.cmp(&right.provider_id))
        });
        product_groups.push(ModelProductGroupView {
            product_group_id,
            billing_kind,
            provider_ids,
            provider_names,
            event_count: accumulator.usage.event_count,
            input_tokens: accumulator.usage.input_tokens,
            output_tokens: accumulator.usage.output_tokens,
            cache_read_tokens: accumulator.usage.cache_read_tokens,
            cache_creation_tokens: accumulator.usage.cache_creation_tokens,
            total_tokens: accumulator.usage.total_tokens,
            total_cost_usd: accumulator.usage.total_cost_usd(),
            cost_source_counts: accumulator.usage.cost_source_counts.clone(),
            models,
        });
    }
    product_groups.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.product_group_id.cmp(&right.product_group_id))
    });

    let mut models = model_accumulators
        .into_iter()
        .map(|(model, accumulator)| model_totals_row(model, accumulator))
        .collect::<Result<Vec<_>, _>>()?;
    models.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.model.cmp(&right.model))
    });

    Ok(ModelUsageDashboardView {
        start_at,
        end_at,
        total_tokens: dashboard_usage.total_tokens,
        total_event_count: dashboard_usage.event_count,
        total_cost_usd: dashboard_usage.total_cost_usd(),
        product_groups,
        models,
        warnings,
    })
}

#[derive(Debug)]
struct AgentProviderAccumulator {
    provider_name: String,
    product_group_id: String,
    billing_kind: BillingKind,
    usage: UsageAccumulator,
}

#[derive(Debug)]
struct AgentAccumulator {
    agent_name: Option<String>,
    archived: bool,
    visible: bool,
    usage: UsageAccumulator,
    providers: BTreeMap<String, AgentProviderAccumulator>,
    models: BTreeMap<String, ModelTotalsAccumulator>,
}

pub fn aggregate_agent_usage(
    db: &Database,
    start_at: i64,
    end_at: i64,
) -> Result<AgentUsageBreakdownView, AppError> {
    let mut dashboard_usage = UsageAccumulator::default();
    let mut agent_accumulators = BTreeMap::<Option<String>, AgentAccumulator>::new();

    for_each_enabled_usage_event(db, start_at, end_at, |event| {
        dashboard_usage.add(&event.sample)?;
        let (agent_name, archived, visible) = if event.agent_module_id.is_none() {
            (None, false, false)
        } else if event.agent_name.is_some() {
            (
                event.agent_name.clone(),
                event.agent_archived_at.is_some(),
                event.agent_visible.unwrap_or(false),
            )
        } else {
            (None, true, false)
        };
        let agent = agent_accumulators
            .entry(event.agent_module_id.clone())
            .or_insert_with(|| AgentAccumulator {
                agent_name,
                archived,
                visible,
                usage: UsageAccumulator::default(),
                providers: BTreeMap::new(),
                models: BTreeMap::new(),
            });
        agent.usage.add(&event.sample)?;
        agent
            .providers
            .entry(event.provider_id.clone())
            .or_insert_with(|| AgentProviderAccumulator {
                provider_name: event.provider_name.clone(),
                product_group_id: event.product_group_id.clone(),
                billing_kind: event.billing_kind,
                usage: UsageAccumulator::default(),
            })
            .usage
            .add(&event.sample)?;

        let model = normalized_model(&event.model);
        let model_total = agent.models.entry(model).or_default();
        model_total.provider_ids.insert(event.provider_id);
        model_total.usage.add(&event.sample)
    })?;

    let mut agents = Vec::with_capacity(agent_accumulators.len());
    for (agent_module_id, accumulator) in agent_accumulators {
        let (first_occurred_at, last_occurred_at) = accumulator.usage.timestamps()?;
        let mut providers = accumulator
            .providers
            .into_iter()
            .map(|(provider_id, provider)| AgentProviderUsageRow {
                provider_id,
                provider_name: provider.provider_name,
                product_group_id: provider.product_group_id,
                billing_kind: provider.billing_kind,
                event_count: provider.usage.event_count,
                input_tokens: provider.usage.input_tokens,
                output_tokens: provider.usage.output_tokens,
                cache_read_tokens: provider.usage.cache_read_tokens,
                cache_creation_tokens: provider.usage.cache_creation_tokens,
                total_tokens: provider.usage.total_tokens,
                total_cost_usd: provider.usage.total_cost_usd(),
                cost_source_counts: provider.usage.cost_source_counts.clone(),
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.provider_id.cmp(&right.provider_id))
        });
        let mut models = accumulator
            .models
            .into_iter()
            .map(|(model, model_accumulator)| model_totals_row(model, model_accumulator))
            .collect::<Result<Vec<_>, _>>()?;
        models.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.model.cmp(&right.model))
        });
        agents.push(AgentUsageRow {
            agent_module_id,
            agent_name: accumulator.agent_name,
            archived: accumulator.archived,
            visible: accumulator.visible,
            event_count: accumulator.usage.event_count,
            input_tokens: accumulator.usage.input_tokens,
            output_tokens: accumulator.usage.output_tokens,
            cache_read_tokens: accumulator.usage.cache_read_tokens,
            cache_creation_tokens: accumulator.usage.cache_creation_tokens,
            total_tokens: accumulator.usage.total_tokens,
            total_cost_usd: accumulator.usage.total_cost_usd(),
            cost_source_counts: accumulator.usage.cost_source_counts.clone(),
            first_occurred_at,
            last_occurred_at,
            providers,
            models,
        });
    }
    agents.sort_by(|left, right| {
        left.agent_module_id
            .is_none()
            .cmp(&right.agent_module_id.is_none())
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| left.agent_module_id.cmp(&right.agent_module_id))
    });

    Ok(AgentUsageBreakdownView {
        start_at,
        end_at,
        total_tokens: dashboard_usage.total_tokens,
        total_event_count: dashboard_usage.event_count,
        total_cost_usd: dashboard_usage.total_cost_usd(),
        agents,
        warnings: vec![],
    })
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
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        total_tokens: 0,
                        total_cost_usd: None,
                        cost_source_counts: CostSourceCounts::default(),
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
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        total_tokens: 0,
                        total_cost_usd: None,
                        cost_source_counts: CostSourceCounts::default(),
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
    use super::{aggregate_agent_usage, aggregate_model_usage, aggregate_provider_range};
    use crate::model::{
        AgentModuleInput, BillingKind, CostSource, TokenSource, UsageEvent, UsageEventLink,
        UsageProviderInput,
    };
    use crate::store::Database;

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
            pricing_origin: None,
            legacy_request_id: None,
            created_at: occurred_at,
        }
    }

    fn dimension_provider(
        id: &str,
        name: &str,
        product_group_id: &str,
        billing_kind: BillingKind,
        enabled: bool,
    ) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: name.to_string(),
            billing_kind,
            product_group_id: product_group_id.to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dimension_event(
        event_id: &str,
        source: TokenSource,
        provider_id: &str,
        agent_module_id: Option<&str>,
        occurred_at: i64,
        model: &str,
        tokens: [u64; 4],
        total_cost_usd: Option<&str>,
        cost_source: CostSource,
    ) -> UsageEvent {
        UsageEvent {
            event_id: event_id.to_string(),
            source,
            provider_id: provider_id.to_string(),
            agent_module_id: agent_module_id.map(str::to_string),
            product_group_id: "event-product-group-is-ignored".to_string(),
            occurred_at,
            model: model.to_string(),
            input_tokens: tokens[0],
            output_tokens: tokens[1],
            cache_read_tokens: tokens[2],
            cache_creation_tokens: tokens[3],
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: None,
            output_cost_usd: None,
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: total_cost_usd.map(str::to_string),
            cost_source,
            pricing_origin: None,
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

    #[test]
    fn dimensions_exclude_linked_duplicates_disabled_providers_and_end_boundary() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&dimension_provider(
            "enabled-provider",
            "Enabled",
            "enabled-product",
            BillingKind::Subscription,
            true,
        ))
        .unwrap();
        db.save_usage_provider(&dimension_provider(
            "disabled-provider",
            "Disabled",
            "disabled-product",
            BillingKind::Metered,
            false,
        ))
        .unwrap();

        let canonical = dimension_event(
            "dimension-canonical",
            TokenSource::Proxy,
            "enabled-provider",
            Some("codex"),
            100,
            "start-model",
            [10, 0, 0, 0],
            Some("0.1"),
            CostSource::Upstream,
        );
        let duplicate = dimension_event(
            "dimension-duplicate",
            TokenSource::SessionLog,
            "enabled-provider",
            Some("codex"),
            101,
            "start-model",
            [1_000, 0, 0, 0],
            Some("9"),
            CostSource::Estimated,
        );
        let inside = dimension_event(
            "dimension-inside",
            TokenSource::Proxy,
            "enabled-provider",
            Some("codex"),
            150,
            "inside-model",
            [0, 5, 0, 0],
            None,
            CostSource::Unavailable,
        );
        let at_end = dimension_event(
            "dimension-at-end",
            TokenSource::Proxy,
            "enabled-provider",
            Some("codex"),
            200,
            "end-model",
            [2_000, 0, 0, 0],
            Some("20"),
            CostSource::Upstream,
        );
        let disabled = dimension_event(
            "dimension-disabled",
            TokenSource::Proxy,
            "disabled-provider",
            Some("codex"),
            125,
            "disabled-model",
            [3_000, 0, 0, 0],
            Some("30"),
            CostSource::Upstream,
        );
        for usage_event in [&canonical, &duplicate, &inside, &at_end, &disabled] {
            db.insert_usage_event(usage_event).unwrap();
        }
        db.insert_usage_event_link(&UsageEventLink {
            canonical_event_id: canonical.event_id.clone(),
            duplicate_event_id: duplicate.event_id.clone(),
            link_kind: "request_id".to_string(),
            link_value: "dimension-shared-request".to_string(),
            created_at: 101,
        })
        .unwrap();

        let model_dashboard = aggregate_model_usage(&db, 100, 200).unwrap();
        assert_eq!(model_dashboard.total_event_count, 2);
        assert_eq!(model_dashboard.total_tokens, 15);
        assert_eq!(model_dashboard.total_cost_usd.as_deref(), Some("0.1"));
        assert_eq!(model_dashboard.product_groups.len(), 1);
        assert_eq!(model_dashboard.product_groups[0].event_count, 2);
        assert_eq!(model_dashboard.models.len(), 2);

        let agent_dashboard = aggregate_agent_usage(&db, 100, 200).unwrap();
        assert_eq!(agent_dashboard.total_event_count, 2);
        assert_eq!(agent_dashboard.total_tokens, 15);
        assert_eq!(agent_dashboard.total_cost_usd.as_deref(), Some("0.1"));
        assert_eq!(agent_dashboard.agents.len(), 1);
        assert_eq!(agent_dashboard.agents[0].event_count, 2);
        assert_eq!(
            agent_dashboard.agents[0].agent_module_id.as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn dimensions_preserve_null_cost_and_sum_decimal_cost_without_float_drift() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&dimension_provider(
            "cost-provider",
            "Costs",
            "cost-product",
            BillingKind::Metered,
            true,
        ))
        .unwrap();
        for usage_event in [
            dimension_event(
                "no-cost",
                TokenSource::Proxy,
                "cost-provider",
                Some("codex"),
                100,
                "no-cost-model",
                [1, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "cost-one",
                TokenSource::Proxy,
                "cost-provider",
                Some("codex"),
                200,
                "priced-model",
                [1, 0, 0, 0],
                Some("0.1"),
                CostSource::Upstream,
            ),
            dimension_event(
                "cost-two",
                TokenSource::Proxy,
                "cost-provider",
                Some("codex"),
                201,
                "priced-model",
                [1, 0, 0, 0],
                Some("0.2"),
                CostSource::Estimated,
            ),
            dimension_event(
                "priced-without-cost",
                TokenSource::Proxy,
                "cost-provider",
                Some("codex"),
                202,
                "priced-model",
                [1, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
        ] {
            db.insert_usage_event(&usage_event).unwrap();
        }

        let no_cost_models = aggregate_model_usage(&db, 100, 101).unwrap();
        assert_eq!(no_cost_models.total_cost_usd, None);
        assert_eq!(no_cost_models.product_groups[0].total_cost_usd, None);
        assert_eq!(no_cost_models.models[0].total_cost_usd, None);
        let no_cost_agents = aggregate_agent_usage(&db, 100, 101).unwrap();
        assert_eq!(no_cost_agents.total_cost_usd, None);
        assert_eq!(no_cost_agents.agents[0].total_cost_usd, None);

        let priced_models = aggregate_model_usage(&db, 200, 203).unwrap();
        assert_eq!(priced_models.total_cost_usd.as_deref(), Some("0.3"));
        assert_eq!(
            priced_models.models[0].total_cost_usd.as_deref(),
            Some("0.3")
        );
        let priced_agents = aggregate_agent_usage(&db, 200, 203).unwrap();
        assert_eq!(priced_agents.total_cost_usd.as_deref(), Some("0.3"));
        assert_eq!(
            priced_agents.agents[0].total_cost_usd.as_deref(),
            Some("0.3")
        );
    }

    #[test]
    fn model_dimension_keeps_provider_leaves_merges_totals_and_sorts_ties() {
        let db = Database::memory().unwrap();
        for provider in [
            dimension_provider(
                "provider-b",
                "Provider B",
                "shared-plan",
                BillingKind::Metered,
                true,
            ),
            dimension_provider(
                "provider-a",
                "Provider A",
                "shared-plan",
                BillingKind::Subscription,
                true,
            ),
        ] {
            db.save_usage_provider(&provider).unwrap();
        }
        for usage_event in [
            dimension_event(
                "shared-b",
                TokenSource::Proxy,
                "provider-b",
                Some("codex"),
                100,
                "shared-model",
                [10, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "shared-a",
                TokenSource::Proxy,
                "provider-a",
                Some("codex"),
                101,
                "shared-model",
                [10, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "model-beta",
                TokenSource::Proxy,
                "provider-a",
                Some("codex"),
                102,
                "beta",
                [5, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "model-alpha",
                TokenSource::Proxy,
                "provider-a",
                Some("codex"),
                103,
                "alpha",
                [5, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "model-unknown",
                TokenSource::Proxy,
                "provider-a",
                Some("codex"),
                104,
                "  \t ",
                [1, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
        ] {
            db.insert_usage_event(&usage_event).unwrap();
        }

        let dashboard = aggregate_model_usage(&db, 100, 200).unwrap();
        assert_eq!(dashboard.warnings, vec!["mixed_billing_kind:shared-plan"]);
        assert_eq!(dashboard.product_groups.len(), 1);
        let product = &dashboard.product_groups[0];
        assert_eq!(product.billing_kind, BillingKind::Metered);
        assert_eq!(product.provider_ids, vec!["provider-a", "provider-b"]);
        assert_eq!(product.provider_names, vec!["Provider A", "Provider B"]);
        let shared_leaves = product
            .models
            .iter()
            .filter(|row| row.model == "shared-model")
            .collect::<Vec<_>>();
        assert_eq!(shared_leaves.len(), 2);
        assert_eq!(shared_leaves[0].provider_id, "provider-a");
        assert_eq!(shared_leaves[1].provider_id, "provider-b");

        assert_eq!(
            dashboard
                .models
                .iter()
                .map(|row| row.model.as_str())
                .collect::<Vec<_>>(),
            vec!["shared-model", "alpha", "beta", "unknown"]
        );
        let shared_total = &dashboard.models[0];
        assert_eq!(shared_total.event_count, 2);
        assert_eq!(shared_total.total_tokens, 20);
        assert_eq!(shared_total.provider_ids, vec!["provider-a", "provider-b"]);
    }

    #[test]
    fn agent_dimension_includes_archived_and_unassigned_and_sorts_nested_ties() {
        let db = Database::memory().unwrap();
        for provider in [
            dimension_provider(
                "agent-provider-b",
                "Agent Provider B",
                "agent-product-b",
                BillingKind::Metered,
                true,
            ),
            dimension_provider(
                "agent-provider-a",
                "Agent Provider A",
                "agent-product-a",
                BillingKind::Subscription,
                true,
            ),
        ] {
            db.save_usage_provider(&provider).unwrap();
        }
        let archived_agent = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Archived Agent".to_string(),
                sort_order: 50,
                visible: true,
            })
            .unwrap();
        for usage_event in [
            dimension_event(
                "archived-agent-event",
                TokenSource::Proxy,
                "agent-provider-a",
                Some(&archived_agent.id),
                100,
                "archived-model",
                [20, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "codex-provider-b",
                TokenSource::Proxy,
                "agent-provider-b",
                Some("codex"),
                101,
                "beta",
                [5, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "codex-provider-a",
                TokenSource::Proxy,
                "agent-provider-a",
                Some("codex"),
                102,
                "alpha",
                [5, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
            dimension_event(
                "unassigned-agent-event",
                TokenSource::Proxy,
                "agent-provider-a",
                None,
                103,
                "unassigned-model",
                [30, 0, 0, 0],
                None,
                CostSource::Unavailable,
            ),
        ] {
            db.insert_usage_event(&usage_event).unwrap();
        }
        db.delete_agent_module(&archived_agent.id).unwrap();

        let dashboard = aggregate_agent_usage(&db, 100, 200).unwrap();
        assert_eq!(dashboard.agents.len(), 3);
        let archived = dashboard
            .agents
            .iter()
            .find(|row| row.agent_module_id.as_deref() == Some(archived_agent.id.as_str()))
            .unwrap();
        assert_eq!(archived.agent_name.as_deref(), Some("Archived Agent"));
        assert!(archived.archived);
        assert!(!archived.visible);

        let codex = dashboard
            .agents
            .iter()
            .find(|row| row.agent_module_id.as_deref() == Some("codex"))
            .unwrap();
        assert_eq!(
            codex
                .providers
                .iter()
                .map(|row| row.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec!["agent-provider-a", "agent-provider-b"]
        );
        assert_eq!(
            codex
                .models
                .iter()
                .map(|row| row.model.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );

        let unassigned = dashboard.agents.last().unwrap();
        assert_eq!(unassigned.agent_module_id, None);
        assert_eq!(unassigned.agent_name, None);
        assert!(!unassigned.archived);
        assert!(!unassigned.visible);
        assert!(unassigned.total_tokens > archived.total_tokens);
    }

    #[test]
    fn dimensions_reject_empty_or_reversed_ranges() {
        let db = Database::memory().unwrap();
        for (start_at, end_at) in [(100, 100), (101, 100)] {
            assert_eq!(
                aggregate_model_usage(&db, start_at, end_at)
                    .unwrap_err()
                    .to_string(),
                "start_at must be before end_at"
            );
            assert_eq!(
                aggregate_agent_usage(&db, start_at, end_at)
                    .unwrap_err()
                    .to_string(),
                "start_at must be before end_at"
            );
        }
    }
}
