use super::aggregation::{
    aggregate_provider_account_range, aggregate_provider_account_trend, most_used_provider_model,
    ProviderRangeAggregate,
};
use super::domain::{
    BillingKind, ManualResetCreditView, QuotaStatusView, UsageProviderView, UsageTrendBucketView,
};
use super::status::{
    classify_metered, classify_subscription_with_thresholds, worst_status, CostQuality,
    SourceClassification, SubscriptionThresholds, UsageStatus,
};
use crate::database::Database;
use crate::error::AppError;
use crate::settings::{ApiBudgetConfig, ApiBudgetMode};
use chrono::{DateTime, Local, LocalResult, NaiveDate, SecondsFormat, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const ROLLING_30_DAYS_SECONDS: i64 = 30 * 86_400;
const INVALID_RESET_TIMESTAMP: &str = "invalid_reset_timestamp";
const RESET_PENDING_REFRESH: &str = "reset_pending_refresh";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayUsageSnapshot {
    pub status: UsageStatus,
    pub generated_at: i64,
    pub last_success_at: Option<i64>,
    pub stale: bool,
    pub refresh_error: Option<String>,
    pub refresh_in_progress: bool,
    pub api_budget: TrayApiBudgetView,
    pub agents: Vec<TrayAgentUsageView>,
}

/// "pro" as the upstream reports it, "Pro" as a person reads it. Only the first
/// letter is touched: "Team" and "Enterprise" are the upstream's own casing.
fn display_plan_label(plan_type: &str) -> String {
    let mut chars = plan_type.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl TrayUsageSnapshot {
    pub fn unknown(generated_at: i64) -> Self {
        Self {
            status: UsageStatus::Unknown,
            generated_at,
            last_success_at: None,
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            api_budget: TrayApiBudgetView::default(),
            agents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayApiBudgetView {
    pub mode: ApiBudgetMode,
    pub provider_count: usize,
    pub today_cost_usd: Option<String>,
    pub daily_budget_usd: Option<String>,
    pub budget_consumed_percent: Option<String>,
    pub cost_quality: CostQuality,
    pub status: UsageStatus,
    pub warning_reason: Option<String>,
}

impl Default for TrayApiBudgetView {
    fn default() -> Self {
        Self {
            mode: ApiBudgetMode::Shared,
            provider_count: 0,
            today_cost_usd: Some("0".to_string()),
            daily_budget_usd: None,
            budget_consumed_percent: None,
            cost_quality: CostQuality::Complete,
            status: UsageStatus::Unknown,
            warning_reason: Some("daily_budget_missing".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgentUsageView {
    pub agent_module_id: String,
    pub name: String,
    pub sort_order: i64,
    pub status: UsageStatus,
    pub providers: Vec<TrayProviderUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayProviderUsageView {
    pub provider_id: String,
    pub provider_name: String,
    pub system_preset_key: Option<String>,
    pub billing_kind: BillingKind,
    pub status: UsageStatus,
    pub warning_reason: Option<String>,
    pub recent_usage: TrayProviderRecentUsageView,
    pub subscription: Option<TraySubscriptionUsageView>,
    pub metered: Option<TrayMeteredUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayProviderRecentUsageView {
    pub start_at: i64,
    pub end_at: i64,
    /// Tokens for the local day, from the same aggregate as `today_cost_usd` so
    /// the two "today" figures can never describe different windows.
    pub today_tokens: u64,
    pub total_tokens: u64,
    pub today_cost_usd: Option<String>,
    pub total_cost_usd: Option<String>,
    pub cost_quality: CostQuality,
    pub most_used_model: Option<String>,
    pub trend_buckets: Vec<UsageTrendBucketView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayQuotaWindowView {
    pub kind: String,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub resets_at: Option<String>,
    pub status: UsageStatus,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySubscriptionUsageView {
    pub plan_label: Option<String>,
    pub windows: Vec<TrayQuotaWindowView>,
    pub manual_resets_remaining: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual_reset_credits: Vec<ManualResetCreditView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayMeteredUsageView {
    pub today_cost_usd: Option<String>,
    pub rolling_30_day_cost_usd: Option<String>,
    pub daily_budget_usd: Option<String>,
    pub budget_consumed_percent: Option<String>,
    pub total_tokens: u64,
    pub cost_quality: CostQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayUsageWindows {
    pub today_start_at: i64,
    pub rolling_30_start_at: i64,
    pub end_at: i64,
    pub next_local_midnight_at: i64,
}

impl TrayUsageWindows {
    pub fn from_local_now(now: DateTime<Local>) -> Option<Self> {
        Self::from_zoned_now(now)
    }

    pub fn from_zoned_now<Tz>(now: DateTime<Tz>) -> Option<Self>
    where
        Tz: TimeZone,
    {
        let now_timestamp = now.timestamp();
        let local_date = now.date_naive();
        let timezone = now.timezone();

        Self::from_calendar_now(now_timestamp, local_date, |date| {
            resolve_local_midnight(&timezone, date)
        })
    }

    fn from_calendar_now(
        now_timestamp: i64,
        local_date: NaiveDate,
        mut resolve_midnight: impl FnMut(NaiveDate) -> Option<i64>,
    ) -> Option<Self> {
        let end_at = now_timestamp.checked_add(1)?;
        let rolling_30_start_at = end_at.checked_sub(ROLLING_30_DAYS_SECONDS)?;
        let next_local_date = local_date.succ_opt()?;
        let today_start_at = resolve_midnight(local_date)?;
        let next_local_midnight_at = resolve_midnight(next_local_date)?;

        if today_start_at > end_at || end_at > next_local_midnight_at {
            return None;
        }

        Some(Self {
            today_start_at,
            rolling_30_start_at,
            end_at,
            next_local_midnight_at,
        })
    }
}

fn resolve_local_midnight<Tz>(timezone: &Tz, date: NaiveDate) -> Option<i64>
where
    Tz: TimeZone,
{
    let midnight = date.and_hms_opt(0, 0, 0)?;
    timestamp_from_local_result(timezone.from_local_datetime(&midnight))
}

fn timestamp_from_local_result<Tz>(result: LocalResult<DateTime<Tz>>) -> Option<i64>
where
    Tz: TimeZone,
{
    match result {
        LocalResult::Single(value) => Some(value.timestamp()),
        LocalResult::Ambiguous(first, second) => Some(first.timestamp().min(second.timestamp())),
        LocalResult::None => None,
    }
}

pub struct TrayUsageProjector {
    db: Arc<Database>,
}

impl TrayUsageProjector {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn project_at(&self, now: DateTime<Local>) -> Result<TrayUsageSnapshot, AppError> {
        self.project_at_with_budget_config(now, None)
    }

    fn project_at_with_budget_config(
        &self,
        now: DateTime<Local>,
        budget_config_override: Option<ApiBudgetConfig>,
    ) -> Result<TrayUsageSnapshot, AppError> {
        let generated_at = now.timestamp();
        let windows = TrayUsageWindows::from_local_now(now)
            .ok_or_else(|| AppError::Message("invalid_tray_usage_windows".to_string()))?;
        let mut providers = self
            .db
            .list_usage_providers()?
            .into_iter()
            .filter(|provider| provider.enabled)
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| {
            (left.name.to_lowercase(), left.id.as_str())
                .cmp(&(right.name.to_lowercase(), right.id.as_str()))
        });
        let settings = crate::settings::get_settings();
        let thresholds = SubscriptionThresholds {
            warning_remaining_percent: settings.usage_warning_remaining_percent,
            critical_remaining_percent: settings.usage_critical_remaining_percent,
        };
        let api_budget_config =
            budget_config_override.unwrap_or_else(|| settings.api_budget_config());
        let projected_providers = self.project_providers(
            &providers,
            windows,
            generated_at,
            thresholds,
            api_budget_config.mode,
        )?;
        let api_budget = project_api_budget(&projected_providers, &api_budget_config);
        let status = worst_status(
            projected_providers
                .iter()
                .map(|provider| provider.status)
                .chain(std::iter::once(api_budget.status)),
        );
        let projected_agents = vec![TrayAgentUsageView {
            agent_module_id: "providers".to_string(),
            name: "Providers".to_string(),
            sort_order: 0,
            status,
            providers: projected_providers,
        }];

        Ok(TrayUsageSnapshot {
            status,
            generated_at,
            last_success_at: Some(generated_at),
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            api_budget,
            agents: projected_agents,
        })
    }

    fn project_providers(
        &self,
        providers: &[UsageProviderView],
        windows: TrayUsageWindows,
        now_timestamp: i64,
        thresholds: SubscriptionThresholds,
        api_budget_mode: ApiBudgetMode,
    ) -> Result<Vec<TrayProviderUsageView>, AppError> {
        providers
            .iter()
            .map(|provider| {
                let rolling_30_day = aggregate_provider_account_range(
                    &self.db,
                    &provider.id,
                    windows.rolling_30_start_at,
                    windows.end_at,
                )?;
                let today = aggregate_provider_account_range(
                    &self.db,
                    &provider.id,
                    windows.today_start_at,
                    windows.end_at,
                )?;
                let (_, trend_buckets) = aggregate_provider_account_trend(
                    &self.db,
                    &provider.id,
                    windows.rolling_30_start_at,
                    windows.end_at,
                )?;
                let recent_usage = TrayProviderRecentUsageView {
                    start_at: windows.rolling_30_start_at,
                    end_at: windows.end_at,
                    today_tokens: checked_total_tokens(&today)?,
                    total_tokens: checked_total_tokens(&rolling_30_day)?,
                    today_cost_usd: display_cost(&today),
                    total_cost_usd: display_cost(&rolling_30_day),
                    cost_quality: cost_quality(&rolling_30_day),
                    most_used_model: most_used_provider_model(
                        &self.db,
                        &provider.id,
                        windows.rolling_30_start_at,
                        windows.end_at,
                    )?,
                    trend_buckets,
                };
                match provider.billing_kind {
                    BillingKind::Subscription => {
                        let (subscription, classification) =
                            self.project_subscription(provider, now_timestamp, thresholds)?;
                        Ok(TrayProviderUsageView {
                            provider_id: provider.id.clone(),
                            provider_name: provider.name.clone(),
                            system_preset_key: provider.system_preset_key.clone(),
                            billing_kind: provider.billing_kind,
                            status: classification.status,
                            warning_reason: classification.reason.map(str::to_string),
                            recent_usage,
                            subscription: Some(subscription),
                            metered: None,
                        })
                    }
                    BillingKind::Metered => {
                        let provider_budget = (api_budget_mode == ApiBudgetMode::PerProvider)
                            .then_some(provider.daily_budget_usd.as_deref())
                            .flatten();
                        let (metered, classification) =
                            self.project_metered(provider_budget, &today, &rolling_30_day)?;
                        Ok(TrayProviderUsageView {
                            provider_id: provider.id.clone(),
                            provider_name: provider.name.clone(),
                            system_preset_key: provider.system_preset_key.clone(),
                            billing_kind: provider.billing_kind,
                            status: classification.status,
                            warning_reason: classification.reason.map(str::to_string),
                            recent_usage,
                            subscription: None,
                            metered: Some(metered),
                        })
                    }
                }
            })
            .collect::<Result<Vec<_>, AppError>>()
    }

    fn project_subscription(
        &self,
        provider: &UsageProviderView,
        now_timestamp: i64,
        thresholds: SubscriptionThresholds,
    ) -> Result<(TraySubscriptionUsageView, SourceClassification), AppError> {
        let snapshot = if provider.quota_source.is_some() {
            self.db.latest_quota_snapshot(&provider.id)?
        } else {
            None
        };
        let five_hour_used = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.five_hour_utilization_percent.as_deref());
        let seven_day_used = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.seven_day_utilization_percent.as_deref());
        let classification =
            classify_subscription_with_thresholds(five_hour_used, seven_day_used, thresholds);
        let windows = [
            (
                "five_hour",
                five_hour_used,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.five_hour_resets_at.as_deref()),
            ),
            (
                "seven_day",
                seven_day_used,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.seven_day_resets_at.as_deref()),
            ),
        ]
        .into_iter()
        .map(|(kind, used, resets_at)| {
            subscription_window(kind, used, resets_at, now_timestamp, thresholds)
        })
        .collect();
        let quota_status = snapshot.as_ref().map(QuotaStatusView::from_snapshot);

        Ok((
            TraySubscriptionUsageView {
                // The popover has always rendered this beside the account name;
                // it had no source until the OAuth token's plan reached the
                // quota snapshot.
                plan_label: quota_status
                    .as_ref()
                    .and_then(|quota| quota.plan_type.as_deref())
                    .map(display_plan_label),
                windows,
                manual_resets_remaining: quota_status
                    .as_ref()
                    .and_then(|quota| quota.manual_resets_remaining),
                manual_reset_credits: quota_status
                    .map(|quota| quota.manual_reset_credits)
                    .unwrap_or_default(),
            },
            classification,
        ))
    }

    fn project_metered(
        &self,
        daily_budget_usd: Option<&str>,
        today: &ProviderRangeAggregate,
        rolling_30_day: &ProviderRangeAggregate,
    ) -> Result<(TrayMeteredUsageView, SourceClassification), AppError> {
        let quality = cost_quality(today);
        let today_cost_usd = display_cost(today);
        let rolling_30_day_cost_usd = display_cost(rolling_30_day);
        let classification = classify_metered(today_cost_usd.as_deref(), daily_budget_usd, quality);
        let total_tokens = checked_total_tokens(rolling_30_day)?;

        Ok((
            TrayMeteredUsageView {
                today_cost_usd,
                rolling_30_day_cost_usd,
                daily_budget_usd: daily_budget_usd.map(str::to_string),
                budget_consumed_percent: classification.consumed_percent.clone(),
                total_tokens,
                cost_quality: quality,
            },
            classification,
        ))
    }
}

fn project_api_budget(
    providers: &[TrayProviderUsageView],
    config: &ApiBudgetConfig,
) -> TrayApiBudgetView {
    let metered = providers
        .iter()
        .filter_map(|provider| provider.metered.as_ref().map(|usage| (provider, usage)))
        .collect::<Vec<_>>();
    let mut total = Decimal::ZERO;
    let mut has_known_cost = false;
    let mut has_estimated = false;
    let mut has_partial_or_unavailable = false;

    for (_, usage) in &metered {
        match usage.cost_quality {
            CostQuality::Estimated => has_estimated = true,
            CostQuality::Partial | CostQuality::Unavailable => has_partial_or_unavailable = true,
            CostQuality::Complete => {}
        }
        match usage
            .today_cost_usd
            .as_deref()
            .and_then(|value| Decimal::from_str_exact(value.trim()).ok())
            .filter(|value| *value >= Decimal::ZERO)
        {
            Some(value) => {
                if let Some(next) = total.checked_add(value) {
                    total = next;
                    has_known_cost = true;
                } else {
                    has_partial_or_unavailable = true;
                }
            }
            None => has_partial_or_unavailable = true,
        }
    }

    let today_cost_usd = if metered.is_empty() || has_known_cost {
        Some(total.normalize().to_string())
    } else {
        None
    };
    let cost_quality = if !metered.is_empty() && !has_known_cost {
        CostQuality::Unavailable
    } else if has_partial_or_unavailable {
        CostQuality::Partial
    } else if has_estimated {
        CostQuality::Estimated
    } else {
        CostQuality::Complete
    };
    let classification = if config.mode == ApiBudgetMode::Shared {
        classify_metered(
            today_cost_usd.as_deref(),
            config.shared_daily_budget_usd.as_deref(),
            cost_quality,
        )
    } else {
        SourceClassification {
            status: worst_status(metered.iter().map(|(provider, _)| provider.status)),
            used_percent: None,
            remaining_percent: None,
            consumed_percent: None,
            reason: None,
        }
    };

    TrayApiBudgetView {
        mode: config.mode,
        provider_count: metered.len(),
        today_cost_usd,
        daily_budget_usd: (config.mode == ApiBudgetMode::Shared)
            .then(|| config.shared_daily_budget_usd.clone())
            .flatten(),
        budget_consumed_percent: classification.consumed_percent,
        cost_quality,
        status: classification.status,
        warning_reason: classification.reason.map(str::to_string),
    }
}

fn subscription_window(
    kind: &str,
    used_percent: Option<&str>,
    resets_at: Option<&str>,
    now_timestamp: i64,
    thresholds: SubscriptionThresholds,
) -> TrayQuotaWindowView {
    let classification = classify_subscription_with_thresholds(used_percent, None, thresholds);
    let (resets_at, reset_reason) = canonical_reset(resets_at, now_timestamp);
    TrayQuotaWindowView {
        kind: kind.to_string(),
        used_percent: classification.used_percent,
        remaining_percent: classification.remaining_percent,
        resets_at,
        status: classification.status,
        unavailable_reason: classification.reason.or(reset_reason).map(str::to_string),
    }
}

fn canonical_reset(
    raw: Option<&str>,
    now_timestamp: i64,
) -> (Option<String>, Option<&'static str>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return (None, Some(INVALID_RESET_TIMESTAMP));
    };
    let reason = (parsed.timestamp() < now_timestamp).then_some(RESET_PENDING_REFRESH);
    let canonical = parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true);
    (Some(canonical), reason)
}

fn display_cost(aggregate: &ProviderRangeAggregate) -> Option<String> {
    if aggregate.event_count == 0 {
        Some("0".to_string())
    } else {
        aggregate.total_cost_usd.clone()
    }
}

fn checked_total_tokens(aggregate: &ProviderRangeAggregate) -> Result<u64, AppError> {
    [
        aggregate.input_tokens,
        aggregate.output_tokens,
        aggregate.cache_read_tokens,
        aggregate.cache_creation_tokens,
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or_else(|| AppError::Database("usage aggregate overflow".to_string()))
    })
}

fn cost_quality(aggregate: &ProviderRangeAggregate) -> CostQuality {
    if aggregate.event_count == 0 {
        CostQuality::Complete
    } else if aggregate.cost_source_counts.unavailable == aggregate.event_count {
        CostQuality::Unavailable
    } else if aggregate.cost_source_counts.unavailable > 0 {
        CostQuality::Partial
    } else if aggregate.cost_source_counts.estimated > 0 {
        CostQuality::Estimated
    } else {
        CostQuality::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::usage::domain::{
        BillingKind, CostSource, QuotaSnapshot, TokenSource, UsageEvent, UsageProviderInput,
    };
    use crate::usage::status::{CostQuality, UsageStatus};
    use chrono::{Datelike, FixedOffset, LocalResult, NaiveDate, TimeZone};
    use rusqlite::params;
    use serde_json::json;
    use std::sync::Arc;

    const DAY_SECONDS: i64 = 86_400;

    fn midnight_at(offset: FixedOffset, date: NaiveDate) -> i64 {
        offset
            .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    fn clean_projector_database() -> Arc<Database> {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("DELETE FROM agent_provider_bindings", [])
                .unwrap();
            conn.execute("UPDATE agent_modules SET visible = 0", [])
                .unwrap();
            conn.execute(
                "UPDATE agent_modules SET visible = 1, sort_order = 0 WHERE id = 'codex'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE agent_modules SET visible = 1, sort_order = 1 WHERE id = 'claude-code'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_modules (
                    id, name, sort_order, visible, is_fixed, archived_at, created_at, updated_at
                 ) VALUES ('hidden-agent', 'Hidden', 2, 0, 0, NULL, 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_modules (
                    id, name, sort_order, visible, is_fixed, archived_at, created_at, updated_at
                 ) VALUES ('archived-agent', 'Archived', 3, 1, 0, 2, 1, 2)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE usage_providers SET enabled = 0", [])
                .unwrap();
        }
        Arc::new(db)
    }

    fn provider(
        id: &str,
        billing_kind: BillingKind,
        quota_source: Option<&str>,
    ) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind,
            product_group_id: format!("group-{id}"),
            token_sources: match billing_kind {
                BillingKind::Subscription => vec![TokenSource::SessionLog],
                BillingKind::Metered => vec![TokenSource::Proxy],
            },
            session_source_bindings: None,
            quota_source: quota_source.map(str::to_string),
            quota_interval_seconds: quota_source.map(|_| 300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn bind(db: &Database, id: &str, agent_id: &str, provider_id: &str, enabled: bool) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_provider_bindings (
                id, agent_module_id, provider_id, enabled,
                api_key_fingerprint, credential_slot, credential_version,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 0, 1, 1)",
            params![id, agent_id, provider_id, enabled],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn event(
        event_id: &str,
        agent_id: &str,
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
            source: TokenSource::Proxy,
            provider_id: provider_id.to_string(),
            agent_module_id: Some(agent_id.to_string()),
            product_group_id: product_group_id.to_string(),
            occurred_at,
            model: "fixture-model".to_string(),
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

    fn quota_snapshot(
        provider_id: &str,
        fetched_at: i64,
        five_hour_used: Option<&str>,
        five_hour_resets_at: Option<&str>,
        seven_day_used: Option<&str>,
        seven_day_resets_at: Option<&str>,
        raw_payload: serde_json::Value,
    ) -> QuotaSnapshot {
        QuotaSnapshot {
            snapshot_id: format!("snapshot-{provider_id}-{fetched_at}"),
            provider_id: provider_id.to_string(),
            fetched_at,
            five_hour_utilization_percent: five_hour_used.map(str::to_string),
            five_hour_resets_at: five_hour_resets_at.map(str::to_string),
            seven_day_utilization_percent: seven_day_used.map(str::to_string),
            seven_day_resets_at: seven_day_resets_at.map(str::to_string),
            manual_resets_remaining: raw_payload
                .pointer("/manualResetCredits/availableCount")
                .and_then(serde_json::Value::as_i64),
            raw_payload,
            created_at: fetched_at,
        }
    }

    fn find_provider<'a>(
        snapshot: &'a TrayUsageSnapshot,
        _agent_id: &str,
        provider_id: &str,
    ) -> &'a TrayProviderUsageView {
        snapshot
            .agents
            .iter()
            .find_map(|agent| {
                agent
                    .providers
                    .iter()
                    .find(|provider| provider.provider_id == provider_id)
            })
            .unwrap_or_else(|| panic!("missing provider {provider_id}"))
    }

    #[test]
    fn projector_lists_enabled_providers_in_dao_order_and_aggregates_by_account() {
        let db = clean_projector_database();
        let now = Local.timestamp_opt(2_000_000_000, 0).single().unwrap();
        let windows = TrayUsageWindows::from_local_now(now).unwrap();
        let future_reset = chrono::DateTime::from_timestamp(now.timestamp() + 3_600, 0)
            .unwrap()
            .to_rfc3339();

        for input in [
            provider("a-subscription", BillingKind::Subscription, Some("fixture")),
            provider("b-metered", BillingKind::Metered, None),
            provider("c-no-budget", BillingKind::Metered, None),
            provider("d-no-source", BillingKind::Subscription, None),
            provider("y-disabled-binding", BillingKind::Metered, None),
            provider("z-disabled-provider", BillingKind::Metered, None),
        ] {
            db.save_usage_provider(&input).unwrap();
        }
        db.set_provider_daily_budget("b-metered", Some("10"))
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers SET enabled = 0 WHERE id = 'z-disabled-provider'",
                [],
            )
            .unwrap();
        }

        // Deliberately insert bindings out of Provider DAO order.
        bind(&db, "codex-d", "codex", "d-no-source", true);
        bind(&db, "codex-z", "codex", "z-disabled-provider", true);
        bind(&db, "codex-c", "codex", "c-no-budget", true);
        bind(&db, "codex-b", "codex", "b-metered", true);
        bind(&db, "codex-a", "codex", "a-subscription", true);
        bind(&db, "codex-y", "codex", "y-disabled-binding", false);
        bind(&db, "claude-b", "claude-code", "b-metered", true);
        bind(&db, "claude-a", "claude-code", "a-subscription", true);
        bind(&db, "hidden-b", "hidden-agent", "b-metered", true);
        bind(&db, "archived-b", "archived-agent", "b-metered", true);

        let session_only = db
            .list_agent_provider_bindings(Some("codex"))
            .unwrap()
            .into_iter()
            .find(|binding| binding.provider_id == "a-subscription")
            .unwrap();
        assert!(session_only.enabled);
        assert!(!session_only.effective_enabled);

        db.append_quota_success(&quota_snapshot(
            "a-subscription",
            now.timestamp() - 10,
            Some("81"),
            Some(&future_reset),
            Some("40"),
            Some(&future_reset),
            json!({
                "manualResetCredits": {
                    "availableCount": 3,
                    "credits": [
                    {
                        "id": "reset-1",
                        "status": "available",
                        "title": "Full reset",
                        "expiresAt": future_reset.clone()
                    }
                    ]
                }
            }),
        ))
        .unwrap();
        db.append_quota_success(&quota_snapshot(
            "d-no-source",
            now.timestamp() - 10,
            Some("ignored-percent-secret-sentinel"),
            Some("ignored-reset-secret"),
            Some("ignored-percent-secret-sentinel"),
            Some("ignored-reset-secret"),
            json!({"ignored": "ignored-payload-secret"}),
        ))
        .unwrap();

        db.insert_usage_event(&event(
            "metered-today",
            "codex",
            "b-metered",
            "group-b-metered",
            now.timestamp(),
            1,
            2,
            3,
            4,
            Some("8.75"),
            CostSource::Upstream,
        ))
        .unwrap();
        db.insert_usage_event(&event(
            "metered-before-today",
            "codex",
            "b-metered",
            "group-b-metered",
            windows.today_start_at - 1,
            10,
            20,
            30,
            40,
            Some("10"),
            CostSource::Estimated,
        ))
        .unwrap();
        db.insert_usage_event(&event(
            "disabled-provider-event",
            "codex",
            "z-disabled-provider",
            "group-z-disabled-provider",
            now.timestamp(),
            9_999,
            9_999,
            9_999,
            9_999,
            Some("9999"),
            CostSource::Upstream,
        ))
        .unwrap();

        let snapshot = TrayUsageProjector::new(db.clone())
            .project_at_with_budget_config(
                now,
                Some(ApiBudgetConfig {
                    mode: ApiBudgetMode::PerProvider,
                    shared_daily_budget_usd: None,
                }),
            )
            .unwrap();

        assert_eq!(snapshot.generated_at, now.timestamp());
        assert_eq!(snapshot.last_success_at, Some(now.timestamp()));
        assert_eq!(snapshot.status, UsageStatus::Red);
        assert_eq!(
            snapshot
                .agents
                .iter()
                .map(|agent| agent.agent_module_id.as_str())
                .collect::<Vec<_>>(),
            vec!["providers"],
        );
        assert_eq!(
            snapshot.agents[0]
                .providers
                .iter()
                .map(|provider| provider.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "a-subscription",
                "b-metered",
                "c-no-budget",
                "d-no-source",
                "y-disabled-binding",
            ],
        );

        let subscription = find_provider(&snapshot, "codex", "a-subscription");
        assert_eq!(subscription.status, UsageStatus::Red);
        let subscription_quota = subscription.subscription.as_ref().unwrap();
        assert_eq!(subscription_quota.plan_label, None);
        assert_eq!(subscription_quota.manual_resets_remaining, Some(3));
        assert_eq!(subscription_quota.manual_reset_credits.len(), 1);
        assert_eq!(
            subscription_quota.manual_reset_credits[0].title.as_deref(),
            Some("Full reset")
        );

        let metered = find_provider(&snapshot, "codex", "b-metered")
            .metered
            .as_ref()
            .unwrap();
        assert_eq!(metered.today_cost_usd.as_deref(), Some("8.75"));
        assert_eq!(metered.rolling_30_day_cost_usd.as_deref(), Some("18.75"),);
        assert_eq!(metered.total_tokens, 110);
        assert_eq!(metered.cost_quality, CostQuality::Complete);
        let recent = &find_provider(&snapshot, "codex", "b-metered").recent_usage;
        assert_eq!(recent.today_cost_usd.as_deref(), Some("8.75"));
        assert_eq!(recent.total_cost_usd.as_deref(), Some("18.75"));
        assert_eq!(recent.total_tokens, 110);
        assert!(recent.most_used_model.is_some());
        assert_eq!(
            recent
                .trend_buckets
                .iter()
                .map(|bucket| bucket.event_count)
                .sum::<u64>(),
            2
        );

        let no_budget = find_provider(&snapshot, "codex", "c-no-budget");
        assert_eq!(no_budget.status, UsageStatus::Unknown);
        assert_eq!(
            no_budget.warning_reason.as_deref(),
            Some("daily_budget_missing")
        );
        assert_eq!(
            no_budget
                .metered
                .as_ref()
                .unwrap()
                .today_cost_usd
                .as_deref(),
            Some("0"),
        );
        assert_eq!(
            no_budget.metered.as_ref().unwrap().cost_quality,
            CostQuality::Complete,
        );

        let no_source = find_provider(&snapshot, "codex", "d-no-source");
        assert_eq!(no_source.status, UsageStatus::Unknown);
        assert_eq!(
            no_source.warning_reason.as_deref(),
            Some("quota_unavailable")
        );
        assert!(no_source
            .subscription
            .as_ref()
            .unwrap()
            .windows
            .iter()
            .all(|window| window.used_percent.is_none() && window.resets_at.is_none()));

        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("ignored-percent-secret-sentinel"));
        assert!(!serialized.contains("ignored-reset-secret"));
        assert!(!serialized.contains("ignored-payload-secret"));

        let shared = TrayUsageProjector::new(db)
            .project_at_with_budget_config(
                now,
                Some(ApiBudgetConfig {
                    mode: ApiBudgetMode::Shared,
                    shared_daily_budget_usd: Some("10".to_string()),
                }),
            )
            .unwrap();
        assert_eq!(shared.api_budget.provider_count, 3);
        assert_eq!(shared.api_budget.today_cost_usd.as_deref(), Some("8.75"));
        assert_eq!(
            shared.api_budget.budget_consumed_percent.as_deref(),
            Some("87.5")
        );
        assert_eq!(shared.api_budget.status, UsageStatus::Red);
        assert!(find_provider(&shared, "codex", "b-metered")
            .metered
            .as_ref()
            .unwrap()
            .daily_budget_usd
            .is_none());
    }

    #[test]
    fn projector_cost_quality_distinguishes_zero_unavailable_partial_and_estimated() {
        let db = clean_projector_database();
        let now = Local.timestamp_opt(2_000_000_000, 0).single().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_modules SET visible = 0 WHERE id = 'claude-code'",
                [],
            )
            .unwrap();
        }

        for id in [
            "a-zero",
            "b-unavailable",
            "c-partial-low",
            "d-partial-high",
            "e-estimated",
        ] {
            db.save_usage_provider(&provider(id, BillingKind::Metered, None))
                .unwrap();
            db.set_provider_daily_budget(id, Some("10")).unwrap();
            bind(&db, &format!("binding-{id}"), "codex", id, true);
        }

        let insert = |event_id: &str,
                      provider_id: &str,
                      total_cost: Option<&str>,
                      source: CostSource,
                      tokens: u64| {
            db.insert_usage_event(&event(
                event_id,
                "codex",
                provider_id,
                &format!("group-{provider_id}"),
                now.timestamp(),
                tokens,
                tokens,
                tokens,
                tokens,
                total_cost,
                source,
            ))
            .unwrap();
        };
        insert(
            "unavailable",
            "b-unavailable",
            None,
            CostSource::Unavailable,
            1,
        );
        insert(
            "partial-low-numeric",
            "c-partial-low",
            Some("4"),
            CostSource::Upstream,
            1,
        );
        insert(
            "partial-low-missing",
            "c-partial-low",
            None,
            CostSource::Unavailable,
            2,
        );
        insert(
            "partial-high-numeric",
            "d-partial-high",
            Some("8"),
            CostSource::Upstream,
            1,
        );
        insert(
            "partial-high-missing",
            "d-partial-high",
            None,
            CostSource::Unavailable,
            2,
        );
        insert(
            "estimated",
            "e-estimated",
            Some("5"),
            CostSource::Estimated,
            1,
        );

        let snapshot = TrayUsageProjector::new(db.clone())
            .project_at_with_budget_config(
                now,
                Some(ApiBudgetConfig {
                    mode: ApiBudgetMode::PerProvider,
                    shared_daily_budget_usd: None,
                }),
            )
            .unwrap();

        let zero = find_provider(&snapshot, "codex", "a-zero");
        assert_eq!(zero.status, UsageStatus::Green);
        assert_eq!(
            zero.metered.as_ref().unwrap().cost_quality,
            CostQuality::Complete
        );
        assert_eq!(
            zero.metered.as_ref().unwrap().today_cost_usd.as_deref(),
            Some("0"),
        );
        assert_eq!(
            zero.metered
                .as_ref()
                .unwrap()
                .rolling_30_day_cost_usd
                .as_deref(),
            Some("0"),
        );

        let unavailable = find_provider(&snapshot, "codex", "b-unavailable");
        assert_eq!(unavailable.status, UsageStatus::Unknown);
        assert_eq!(
            unavailable.metered.as_ref().unwrap().cost_quality,
            CostQuality::Unavailable,
        );
        assert_eq!(unavailable.metered.as_ref().unwrap().today_cost_usd, None);

        let partial_low = find_provider(&snapshot, "codex", "c-partial-low");
        assert_eq!(partial_low.status, UsageStatus::Unknown);
        assert_eq!(partial_low.warning_reason.as_deref(), Some("partial_cost"));
        assert_eq!(
            partial_low.metered.as_ref().unwrap().cost_quality,
            CostQuality::Partial,
        );

        let partial_high = find_provider(&snapshot, "codex", "d-partial-high");
        assert_eq!(partial_high.status, UsageStatus::Red);
        assert_eq!(partial_high.warning_reason.as_deref(), Some("partial_cost"));
        assert_eq!(partial_high.metered.as_ref().unwrap().total_tokens, 12);

        let estimated = find_provider(&snapshot, "codex", "e-estimated");
        assert_eq!(estimated.status, UsageStatus::Yellow);
        assert_eq!(
            estimated.metered.as_ref().unwrap().cost_quality,
            CostQuality::Estimated,
        );
        assert_eq!(snapshot.status, UsageStatus::Red);
    }

    #[test]
    fn projector_canonicalizes_resets_and_never_serializes_persisted_secrets() {
        let db = clean_projector_database();
        let now = Local.timestamp_opt(2_000_000_000, 0).single().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_modules SET visible = 0 WHERE id = 'claude-code'",
                [],
            )
            .unwrap();
        }

        let mut private_provider = provider(
            "a-private-subscription",
            BillingKind::Subscription,
            Some("fixture"),
        );
        private_provider.route_config = Some(json!({
            "api_key": "route-config-secret-sentinel"
        }));
        private_provider.quota_config = Some(json!({
            "token": "quota-config-secret-sentinel"
        }));
        db.save_usage_provider(&private_provider).unwrap();
        bind(
            &db,
            "private-binding",
            "codex",
            "a-private-subscription",
            true,
        );
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?2, credential_slot = ?3,
                     credential_version = 1, route_protocol = ?4
                 WHERE id = ?1",
                params![
                    "private-binding",
                    vec![0x41_u8; 32],
                    "binding-credential-secret-sentinel",
                    "binding-route-secret-sentinel",
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO provider_api_credentials (
                    provider_id, api_key_fingerprint, credential_slot,
                    credential_version, last_test_at, last_test_status,
                    last_test_error_code, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 1, 1, 'failed', ?4, 1, 1)",
                params![
                    "a-private-subscription",
                    vec![0x42_u8; 32],
                    "provider-credential-secret-sentinel",
                    "provider-test-secret-sentinel",
                ],
            )
            .unwrap();
        }

        let past_reset = chrono::DateTime::from_timestamp(now.timestamp() - 1, 0)
            .unwrap()
            .to_rfc3339();
        db.append_quota_success(&quota_snapshot(
            "a-private-subscription",
            now.timestamp() - 10,
            Some("50"),
            Some(&past_reset),
            Some("55"),
            Some("reset-secret-sentinel"),
            json!({"token": "raw-payload-secret-sentinel"}),
        ))
        .unwrap();
        db.record_quota_failure(
            "a-private-subscription",
            now.timestamp(),
            "quota-fetch-error-secret-sentinel",
        )
        .unwrap();

        let snapshot = TrayUsageProjector::new(db.clone()).project_at(now).unwrap();
        let provider = find_provider(&snapshot, "codex", "a-private-subscription");
        let subscription = provider.subscription.as_ref().unwrap();
        assert_eq!(
            subscription.plan_label, None,
            "a payload without a plan must not invent one"
        );
        assert_eq!(subscription.windows.len(), 2);
        assert_eq!(
            subscription.windows[0].unavailable_reason.as_deref(),
            Some("reset_pending_refresh"),
        );
        assert!(subscription.windows[0]
            .resets_at
            .as_deref()
            .is_some_and(|value| value.ends_with('Z')));
        assert_eq!(subscription.windows[1].resets_at, None);
        assert_eq!(
            subscription.windows[1].unavailable_reason.as_deref(),
            Some("invalid_reset_timestamp"),
        );
        let exact_now_reset = chrono::DateTime::from_timestamp(now.timestamp(), 0)
            .unwrap()
            .to_rfc3339();
        let (canonical_exact_now, exact_now_reason) =
            canonical_reset(Some(&exact_now_reset), now.timestamp());
        assert!(canonical_exact_now.is_some());
        assert_eq!(exact_now_reason, None);
        let invalid_usage_with_past_reset = subscription_window(
            "fixture",
            Some("invalid-percent-secret-sentinel"),
            Some(&past_reset),
            now.timestamp(),
            SubscriptionThresholds::default(),
        );
        assert_eq!(
            invalid_usage_with_past_reset.unavailable_reason.as_deref(),
            Some("invalid_quota_percent"),
        );
        assert!(!serde_json::to_string(&invalid_usage_with_past_reset)
            .unwrap()
            .contains("invalid-percent-secret-sentinel"));

        let serialized = serde_json::to_string(&snapshot).unwrap();
        for secret in [
            "route-config-secret-sentinel",
            "quota-config-secret-sentinel",
            "binding-credential-secret-sentinel",
            "binding-route-secret-sentinel",
            "provider-credential-secret-sentinel",
            "provider-test-secret-sentinel",
            "raw-payload-secret-sentinel",
            "quota-fetch-error-secret-sentinel",
            "reset-secret-sentinel",
        ] {
            assert!(!serialized.contains(secret), "{secret}: {serialized}");
        }
    }

    #[test]
    fn ordinary_day_windows_are_half_open_and_include_the_current_second() {
        let offset = FixedOffset::east_opt(8 * 3_600).unwrap();
        let now = offset
            .with_ymd_and_hms(2026, 7, 16, 12, 30, 45)
            .single()
            .unwrap();

        let windows = TrayUsageWindows::from_zoned_now(now).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();

        assert_eq!(windows.today_start_at, midnight_at(offset, today));
        assert_eq!(windows.end_at, now.timestamp() + 1);
        assert_eq!(
            windows.next_local_midnight_at,
            midnight_at(offset, tomorrow)
        );
        assert_eq!(
            windows.end_at - windows.rolling_30_start_at,
            30 * DAY_SECONDS,
        );
        assert!(windows.today_start_at <= now.timestamp());
        assert!(now.timestamp() < windows.end_at);
        assert!(windows.end_at <= windows.next_local_midnight_at);
    }

    #[test]
    fn last_second_of_the_day_uses_next_midnight_as_the_exclusive_end() {
        let offset = FixedOffset::east_opt(8 * 3_600).unwrap();
        let now = offset
            .with_ymd_and_hms(2026, 7, 16, 23, 59, 59)
            .single()
            .unwrap();

        let windows = TrayUsageWindows::from_zoned_now(now).unwrap();

        assert_eq!(windows.end_at, now.timestamp() + 1);
        assert_eq!(windows.end_at, windows.next_local_midnight_at);
        assert!(windows.end_at <= windows.next_local_midnight_at);
    }

    #[test]
    fn spring_forward_uses_the_next_local_calendar_midnight() {
        let standard = FixedOffset::west_opt(5 * 3_600).unwrap();
        let daylight = FixedOffset::west_opt(4 * 3_600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 3, 9).unwrap();
        let now = daylight
            .with_ymd_and_hms(2026, 3, 8, 12, 30, 0)
            .single()
            .unwrap();

        let windows =
            TrayUsageWindows::from_calendar_now(now.timestamp(), today, |date| match date {
                value if value == today => Some(midnight_at(standard, value)),
                value if value == tomorrow => Some(midnight_at(daylight, value)),
                _ => None,
            })
            .unwrap();

        assert_eq!(windows.today_start_at, 1_772_946_000);
        assert_eq!(windows.next_local_midnight_at, 1_773_028_800);
        assert_eq!(
            windows.next_local_midnight_at - windows.today_start_at,
            23 * 3_600,
        );
        assert_ne!(
            windows.next_local_midnight_at,
            windows.today_start_at + DAY_SECONDS,
        );
        assert_eq!(windows.end_at, now.timestamp() + 1);
    }

    #[test]
    fn fall_back_uses_the_next_local_calendar_midnight() {
        let daylight = FixedOffset::west_opt(4 * 3_600).unwrap();
        let standard = FixedOffset::west_opt(5 * 3_600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let now = standard
            .with_ymd_and_hms(2026, 11, 1, 12, 30, 0)
            .single()
            .unwrap();

        let windows =
            TrayUsageWindows::from_calendar_now(now.timestamp(), today, |date| match date {
                value if value == today => Some(midnight_at(daylight, value)),
                value if value == tomorrow => Some(midnight_at(standard, value)),
                _ => None,
            })
            .unwrap();

        assert_eq!(windows.today_start_at, 1_793_505_600);
        assert_eq!(windows.next_local_midnight_at, 1_793_595_600);
        assert_eq!(
            windows.next_local_midnight_at - windows.today_start_at,
            25 * 3_600,
        );
        assert_ne!(
            windows.next_local_midnight_at,
            windows.today_start_at + DAY_SECONDS,
        );
        assert_eq!(
            windows.end_at - windows.rolling_30_start_at,
            30 * DAY_SECONDS,
        );
        assert!(windows.next_local_midnight_at > windows.end_at);
    }

    #[test]
    fn calendar_window_fails_when_either_local_midnight_cannot_be_resolved() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();

        assert!(TrayUsageWindows::from_calendar_now(1_000, today, |_| None).is_none());
    }

    #[test]
    fn calendar_window_rejects_timestamp_and_date_overflow() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
        let midnight = 1_000;

        assert!(
            TrayUsageWindows::from_calendar_now(i64::MAX, today, |_| { Some(midnight) }).is_none()
        );
        assert!(
            TrayUsageWindows::from_calendar_now(i64::MIN, today, |_| { Some(midnight) }).is_none()
        );
        assert!(
            TrayUsageWindows::from_calendar_now(0, NaiveDate::MAX, |_| { Some(midnight) })
                .is_none()
        );
    }

    #[test]
    fn ambiguous_midnight_uses_the_earliest_instant_and_gap_returns_none() {
        let offset = FixedOffset::east_opt(0).unwrap();
        let earlier = offset.timestamp_opt(1_000, 0).single().unwrap();
        let later = offset.timestamp_opt(2_000, 0).single().unwrap();

        assert_eq!(
            timestamp_from_local_result(LocalResult::Ambiguous(later, earlier)),
            Some(1_000),
        );

        let gap: LocalResult<chrono::DateTime<FixedOffset>> = LocalResult::None;
        assert_eq!(timestamp_from_local_result(gap), None);
    }

    #[test]
    fn unknown_snapshot_has_safe_empty_defaults() {
        let snapshot = TrayUsageSnapshot::unknown(123);

        assert_eq!(snapshot.status, UsageStatus::Unknown);
        assert_eq!(snapshot.generated_at, 123);
        assert_eq!(snapshot.last_success_at, None);
        assert!(!snapshot.stale);
        assert_eq!(snapshot.refresh_error, None);
        assert!(!snapshot.refresh_in_progress);
        assert!(snapshot.agents.is_empty());
    }

    #[test]
    fn stale_snapshot_can_retain_a_known_alert() {
        let snapshot = TrayUsageSnapshot {
            status: UsageStatus::Red,
            generated_at: 200,
            last_success_at: Some(100),
            stale: true,
            refresh_error: Some("refresh_failed".to_string()),
            refresh_in_progress: false,
            api_budget: TrayApiBudgetView::default(),
            agents: Vec::new(),
        };

        assert_eq!(snapshot.status, UsageStatus::Red);
        assert!(snapshot.stale);
        assert_eq!(snapshot.last_success_at, Some(100));
    }

    #[test]
    fn tray_snapshot_serializes_only_presentation_safe_fields() {
        let snapshot = TrayUsageSnapshot {
            status: UsageStatus::Yellow,
            generated_at: 200,
            last_success_at: Some(190),
            stale: false,
            refresh_error: None,
            refresh_in_progress: true,
            api_budget: TrayApiBudgetView::default(),
            agents: vec![TrayAgentUsageView {
                agent_module_id: "codex".to_string(),
                name: "Codex".to_string(),
                sort_order: 1,
                status: UsageStatus::Yellow,
                providers: vec![TrayProviderUsageView {
                    provider_id: "openai".to_string(),
                    provider_name: "OpenAI".to_string(),
                    system_preset_key: Some("openai".to_string()),
                    billing_kind: BillingKind::Metered,
                    status: UsageStatus::Yellow,
                    warning_reason: Some("partial_cost".to_string()),
                    recent_usage: TrayProviderRecentUsageView {
                        start_at: 1,
                        end_at: 200,
                        today_tokens: 7,
                        total_tokens: 42,
                        today_cost_usd: Some("5".to_string()),
                        total_cost_usd: Some("80".to_string()),
                        cost_quality: CostQuality::Partial,
                        most_used_model: Some("gpt-5.6-sol".to_string()),
                        trend_buckets: vec![UsageTrendBucketView {
                            start_at: 100,
                            end_at: 200,
                            event_count: 1,
                            input_tokens: 10,
                            output_tokens: 2,
                            cache_read_tokens: 30,
                            cache_creation_tokens: 0,
                            total_tokens: 42,
                            total_cost_usd: Some("5".to_string()),
                            cost_source_counts: super::super::domain::CostSourceCounts {
                                upstream: 0,
                                estimated: 1,
                                unavailable: 0,
                            },
                        }],
                    },
                    subscription: Some(TraySubscriptionUsageView {
                        plan_label: Some("Plus".to_string()),
                        windows: vec![TrayQuotaWindowView {
                            kind: "five_hour".to_string(),
                            used_percent: Some("50".to_string()),
                            remaining_percent: Some("50".to_string()),
                            resets_at: Some("2026-07-16T13:00:00Z".to_string()),
                            status: UsageStatus::Yellow,
                            unavailable_reason: None,
                        }],
                        manual_resets_remaining: None,
                        manual_reset_credits: Vec::new(),
                    }),
                    metered: Some(TrayMeteredUsageView {
                        today_cost_usd: Some("5".to_string()),
                        rolling_30_day_cost_usd: Some("80".to_string()),
                        daily_budget_usd: Some("10".to_string()),
                        budget_consumed_percent: Some("50".to_string()),
                        total_tokens: 42,
                        cost_quality: CostQuality::Partial,
                    }),
                }],
            }],
        };

        let value = serde_json::to_value(&snapshot).unwrap();
        let recent_usage_json = json!({
            "startAt": 1,
            "endAt": 200,
            "todayTokens": 7,
            "totalTokens": 42,
            "todayCostUsd": "5",
            "totalCostUsd": "80",
            "costQuality": "partial",
            "mostUsedModel": "gpt-5.6-sol",
            "trendBuckets": [{
                "startAt": 100,
                "endAt": 200,
                "eventCount": 1,
                "inputTokens": 10,
                "outputTokens": 2,
                "cacheReadTokens": 30,
                "cacheCreationTokens": 0,
                "totalTokens": 42,
                "totalCostUsd": "5",
                "costSourceCounts": {
                    "upstream": 0,
                    "estimated": 1,
                    "unavailable": 0
                }
            }]
        });
        assert_eq!(
            value,
            json!({
                "status": "yellow",
                "generatedAt": 200,
                "lastSuccessAt": 190,
                "stale": false,
                "refreshError": null,
                "refreshInProgress": true,
                "apiBudget": {
                    "mode": "shared",
                    "providerCount": 0,
                    "todayCostUsd": "0",
                    "dailyBudgetUsd": null,
                    "budgetConsumedPercent": null,
                    "costQuality": "complete",
                    "status": "unknown",
                    "warningReason": "daily_budget_missing"
                },
                "agents": [{
                    "agentModuleId": "codex",
                    "name": "Codex",
                    "sortOrder": 1,
                    "status": "yellow",
                    "providers": [{
                        "providerId": "openai",
                        "providerName": "OpenAI",
                        "systemPresetKey": "openai",
                        "billingKind": "metered",
                        "status": "yellow",
                        "warningReason": "partial_cost",
                        "recentUsage": recent_usage_json,
                        "subscription": {
                            "planLabel": "Plus",
                            "windows": [{
                                "kind": "five_hour",
                                "usedPercent": "50",
                                "remainingPercent": "50",
                                "resetsAt": "2026-07-16T13:00:00Z",
                                "status": "yellow",
                                "unavailableReason": null
                            }],
                            "manualResetsRemaining": null
                        },
                        "metered": {
                            "todayCostUsd": "5",
                            "rolling30DayCostUsd": "80",
                            "dailyBudgetUsd": "10",
                            "budgetConsumedPercent": "50",
                            "totalTokens": 42,
                            "costQuality": "partial"
                        }
                    }]
                }]
            }),
        );

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let round_trip: TrayUsageSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip, snapshot);
        assert!(serialized.contains("\"generatedAt\""));
        assert!(serialized.contains("\"agentModuleId\""));
        assert!(serialized.contains("\"rolling30DayCostUsd\""));
        for forbidden in [
            "apiKey",
            "oauthToken",
            "quotaConfig",
            "routeConfig",
            "credentialFingerprint",
            "rawPayload",
            "rawError",
            "lastError",
        ] {
            assert!(!serialized.contains(forbidden), "{forbidden}: {serialized}");
        }
    }
    #[test]
    fn subscription_plan_label_comes_from_the_quota_snapshot() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&provider(
            "a-subscription",
            BillingKind::Subscription,
            Some("fixture"),
        ))
        .unwrap();
        bind(&db, "plan-binding", "codex", "a-subscription", true);
        let now = chrono::Local::now();
        db.append_quota_success(&quota_snapshot(
            "a-subscription",
            now.timestamp() - 10,
            Some("20"),
            None,
            Some("30"),
            None,
            // The tray reads the same lifted field the dashboard does; the rest
            // of the payload must not reach the popover.
            json!({"planType": "pro", "secret": "must-not-leak"}),
        ))
        .unwrap();

        let snapshot = TrayUsageProjector::new(db.clone()).project_at(now).unwrap();
        let provider = find_provider(&snapshot, "codex", "a-subscription");
        let subscription = provider.subscription.as_ref().unwrap();
        assert_eq!(subscription.plan_label.as_deref(), Some("Pro"));
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("must-not-leak"));
    }

    #[test]
    fn plan_label_only_touches_the_first_letter() {
        assert_eq!(display_plan_label("pro"), "Pro");
        assert_eq!(display_plan_label("plus"), "Plus");
        // Upstream casing beyond the first letter is theirs, not ours.
        assert_eq!(display_plan_label("enterprise"), "Enterprise");
        assert_eq!(display_plan_label(""), "");
    }
}
