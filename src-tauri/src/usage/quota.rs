use crate::credentials::codex_oauth_auth::CodexOAuthManager;
use crate::database::Database;
use crate::error::AppError;
use crate::services::coding_plan::get_coding_plan_quota;
use crate::services::subscription::{
    get_subscription_quota, query_managed_codex_oauth_quota, SubscriptionQuota, TIER_FIVE_HOUR,
    TIER_SEVEN_DAY, TIER_WEEKLY_LIMIT,
};
use crate::usage::domain::{
    BillingKind, QuotaFetchState, QuotaSnapshot, QuotaStatusView, UsageProviderStored,
};
use crate::usage::system_providers::{CLAUDE_LOCAL_QUOTA_SOURCE, MANAGED_CODEX_QUOTA_SOURCE};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{watch, Mutex as AsyncMutex, RwLock};
use uuid::Uuid;

pub const DEFAULT_QUOTA_INTERVAL_SECONDS: u64 = 300;
const SCHEDULER_TICK_SECONDS: u64 = 30;
const FAILURE_RETRY_DELAYS_SECONDS: [u64; 5] = [30, 60, 300, 600, 1_800];

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedQuota {
    pub five_hour_utilization_percent: Option<String>,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_utilization_percent: Option<String>,
    pub seven_day_resets_at: Option<String>,
    pub manual_resets_remaining: Option<i64>,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRefreshResult {
    pub snapshot: QuotaStatusView,
    pub fetch_state: QuotaFetchState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuotaSchedulerCycle {
    pub attempted: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaSchedulerOutcome {
    Completed { attempted: u32, had_errors: bool },
    Failed,
}

pub type QuotaCycleCallback =
    Arc<dyn Fn(QuotaSchedulerOutcome) -> BoxFuture<'static, ()> + Send + Sync + 'static>;

/// Object-safe boundary around existing quota HTTP/parsing implementations.
/// The adapter returns their common `SubscriptionQuota` representation so this
/// module only owns normalization and persistence semantics.
pub trait QuotaCollector: Send + Sync {
    fn source(&self) -> &'static str;

    fn collect<'a>(
        &'a self,
        provider: &'a UsageProviderStored,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>>;
}

struct SubscriptionQuotaCollector {
    source: &'static str,
}

impl QuotaCollector for SubscriptionQuotaCollector {
    fn source(&self) -> &'static str {
        self.source
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move { get_subscription_quota(self.source).await })
    }
}

struct ClaudeLocalQuotaCollector;

impl QuotaCollector for ClaudeLocalQuotaCollector {
    fn source(&self) -> &'static str {
        CLAUDE_LOCAL_QUOTA_SOURCE
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move { crate::claude_quota::collect_local_quota() })
    }
}

struct CodingPlanQuotaCollector;

impl QuotaCollector for CodingPlanQuotaCollector {
    fn source(&self) -> &'static str {
        "coding_plan"
    }

    fn collect<'a>(
        &'a self,
        provider: &'a UsageProviderStored,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move {
            let empty_config = Value::Null;
            let config = provider.quota_config.as_ref().unwrap_or(&empty_config);
            let string = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| config.get(key).and_then(Value::as_str))
                    .unwrap_or("")
            };
            let optional = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| config.get(key).and_then(Value::as_str))
                    .filter(|value| !value.trim().is_empty())
            };
            get_coding_plan_quota(
                string(&["base_url", "baseUrl"]),
                string(&["api_key", "apiKey"]),
                optional(&["coding_plan_provider", "codingPlanProvider"]),
                optional(&["access_key_id", "accessKeyId"]),
                optional(&["secret_access_key", "secretAccessKey"]),
                optional(&["team_organization_id", "teamOrganizationId"]),
                optional(&["team_project_id", "teamProjectId"]),
            )
            .await
        })
    }
}

struct ManagedCodexOAuthQuotaCollector {
    manager: Arc<RwLock<CodexOAuthManager>>,
}

impl QuotaCollector for ManagedCodexOAuthQuotaCollector {
    fn source(&self) -> &'static str {
        MANAGED_CODEX_QUOTA_SOURCE
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move { query_managed_codex_oauth_quota(&self.manager, None).await })
    }
}

#[derive(Clone)]
pub struct QuotaService {
    db: Arc<Database>,
    collectors: Arc<HashMap<String, Arc<dyn QuotaCollector>>>,
    in_flight: Arc<AsyncMutex<HashMap<String, Arc<QuotaFlight>>>>,
}

type SharedQuotaRefreshResult = Result<QuotaRefreshResult, String>;

struct QuotaFlight {
    result: watch::Sender<Option<SharedQuotaRefreshResult>>,
}

impl QuotaService {
    pub fn new(db: Arc<Database>) -> Self {
        Self::with_collectors(db, Self::legacy_collectors())
    }

    pub fn production(
        db: Arc<Database>,
        codex_oauth_manager: Arc<RwLock<CodexOAuthManager>>,
    ) -> Self {
        let mut collectors = Self::legacy_collectors();
        collectors.push(Arc::new(ManagedCodexOAuthQuotaCollector {
            manager: codex_oauth_manager,
        }));
        Self::with_collectors(db, collectors)
    }

    fn legacy_collectors() -> Vec<Arc<dyn QuotaCollector>> {
        vec![
            Arc::new(SubscriptionQuotaCollector { source: "claude" }),
            Arc::new(SubscriptionQuotaCollector { source: "codex" }),
            Arc::new(ClaudeLocalQuotaCollector),
            Arc::new(CodingPlanQuotaCollector),
        ]
    }

    pub fn with_collectors(db: Arc<Database>, collectors: Vec<Arc<dyn QuotaCollector>>) -> Self {
        let collectors = collectors
            .into_iter()
            .map(|collector| (collector.source().to_string(), collector))
            .collect();
        Self {
            db,
            collectors: Arc::new(collectors),
            in_flight: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    pub async fn refresh_provider(
        &self,
        provider_id: &str,
    ) -> Result<QuotaRefreshResult, AppError> {
        self.refresh_provider_at(provider_id, now_timestamp()?)
            .await
    }

    async fn refresh_provider_at(
        &self,
        provider_id: &str,
        attempted_at: i64,
    ) -> Result<QuotaRefreshResult, AppError> {
        let (flight, is_leader) = {
            let mut in_flight = self.in_flight.lock().await;
            if let Some(flight) = in_flight.get(provider_id) {
                (flight.clone(), false)
            } else {
                let (result, _) = watch::channel(None);
                let flight = Arc::new(QuotaFlight { result });
                in_flight.insert(provider_id.to_string(), flight.clone());
                (flight, true)
            }
        };

        if is_leader {
            let service = self.clone();
            let provider_id = provider_id.to_string();
            let leader_flight = flight.clone();
            tokio::spawn(async move {
                let result = service
                    .refresh_provider_once_at(&provider_id, attempted_at)
                    .await
                    .map_err(|error| error.to_string());
                leader_flight.result.send_replace(Some(result));

                let mut in_flight = service.in_flight.lock().await;
                if in_flight
                    .get(&provider_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &leader_flight))
                {
                    in_flight.remove(&provider_id);
                }
            });
        }

        let mut result = flight.result.subscribe();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result.map_err(AppError::Message);
            }
            result.changed().await.map_err(|_| {
                AppError::Message("quota refresh singleflight unavailable".to_string())
            })?;
        }
    }

    async fn refresh_provider_once_at(
        &self,
        provider_id: &str,
        attempted_at: i64,
    ) -> Result<QuotaRefreshResult, AppError> {
        let provider = self
            .db
            .get_usage_provider(provider_id)?
            .ok_or_else(|| AppError::Message("usage provider not found".to_string()))?;
        validate_refresh_provider(&provider)?;

        let normalized = match self.collect_normalized_quota(&provider).await {
            Ok(normalized) => normalized,
            Err(error) => {
                self.db
                    .record_quota_failure(provider_id, attempted_at, &error.to_string())?;
                return Err(error);
            }
        };

        let snapshot = QuotaSnapshot {
            snapshot_id: Uuid::new_v4().to_string(),
            provider_id: provider.id,
            fetched_at: attempted_at,
            five_hour_utilization_percent: normalized.five_hour_utilization_percent,
            five_hour_resets_at: normalized.five_hour_resets_at,
            seven_day_utilization_percent: normalized.seven_day_utilization_percent,
            seven_day_resets_at: normalized.seven_day_resets_at,
            manual_resets_remaining: normalized.manual_resets_remaining,
            raw_payload: normalized.raw_payload,
            created_at: attempted_at,
        };
        let fetch_state = self.db.append_quota_success(&snapshot)?;
        Ok(QuotaRefreshResult {
            snapshot: quota_status_view(&snapshot),
            fetch_state,
        })
    }

    async fn collect_normalized_quota(
        &self,
        provider: &UsageProviderStored,
    ) -> Result<NormalizedQuota, AppError> {
        let quota_source = provider
            .quota_source
            .as_deref()
            .ok_or_else(|| AppError::Message("quota source is not configured".to_string()))?;
        self.validate_local_quota_source_ownership(provider, quota_source)?;
        let collector = self.collectors.get(quota_source).ok_or_else(|| {
            AppError::Message(format!("unsupported quota source: {quota_source}"))
        })?;

        let quota = match collector.collect(provider).await {
            Ok(quota) if quota.success => quota,
            Ok(quota) => {
                let error = quota
                    .error
                    .or(quota.credential_message)
                    .unwrap_or_else(|| "quota collection failed".to_string());
                return Err(AppError::Message(error));
            }
            Err(error) => return Err(AppError::Message(error)),
        };

        let manual_resets_remaining = provider
            .quota_config
            .as_ref()
            .and_then(manual_resets_from_config);
        normalize_subscription_quota(&quota, manual_resets_remaining)
    }

    fn validate_local_quota_source_ownership(
        &self,
        provider: &UsageProviderStored,
        quota_source: &str,
    ) -> Result<(), AppError> {
        if !matches!(quota_source, "claude" | "codex" | CLAUDE_LOCAL_QUOTA_SOURCE)
            && quota_source != MANAGED_CODEX_QUOTA_SOURCE
        {
            return Ok(());
        }
        let conflict = self.db.list_usage_providers()?.into_iter().any(|other| {
            other.id != provider.id
                && other.enabled
                && other.billing_kind == BillingKind::Subscription
                && other.quota_source.as_deref() == Some(quota_source)
        });
        if conflict {
            return Err(AppError::Message(format!(
                "local quota source {quota_source} must belong to exactly one enabled provider"
            )));
        }
        Ok(())
    }

    /// Runs one deterministic scheduling pass. Normal refreshes use the
    /// configured interval. Failures retry after 30s, 1m, 5m, 10m, then 30m;
    /// the fetch state only becomes stale when the 10-minute retry fails.
    pub async fn refresh_due_at(&self, now: i64) -> Result<QuotaSchedulerCycle, AppError> {
        let mut cycle = QuotaSchedulerCycle::default();
        for view in self.db.list_usage_providers()? {
            if !view.enabled
                || view.billing_kind != BillingKind::Subscription
                || view.quota_source.is_none()
            {
                continue;
            }
            let Some(interval) = quota_interval_seconds(view.quota_interval_seconds)? else {
                continue;
            };
            let state = self.db.get_quota_fetch_state(&view.id)?;
            let due = state.as_ref().is_none_or(|state| {
                let Some(last_attempt) = state.last_attempt_at else {
                    return true;
                };
                let delay = quota_refresh_delay_seconds(state, interval);
                now.saturating_sub(last_attempt) >= delay as i64
            });
            if !due {
                continue;
            }
            cycle.attempted += 1;
            if let Err(error) = self.refresh_provider_at(&view.id, now).await {
                cycle.errors.push(format!("{}: {error}", view.id));
            }
        }
        Ok(cycle)
    }

    async fn run_scheduler_cycle_at(&self, now: i64, after_cycle: &QuotaCycleCallback) {
        let outcome = match self.refresh_due_at(now).await {
            Ok(cycle) => {
                let had_errors = !cycle.errors.is_empty();
                if had_errors {
                    log::warn!("quota scheduler refresh failed");
                }
                QuotaSchedulerOutcome::Completed {
                    attempted: cycle.attempted,
                    had_errors,
                }
            }
            Err(_) => {
                log::warn!("quota scheduler cycle failed");
                QuotaSchedulerOutcome::Failed
            }
        };
        after_cycle(outcome).await;
    }

    pub fn start_scheduler(
        self: Arc<Self>,
        after_cycle: QuotaCycleCallback,
    ) -> QuotaSchedulerHandle {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let task = tauri::async_runtime::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(SCHEDULER_TICK_SECONDS));
            interval.tick().await;
            loop {
                tokio::select! {
                    changed = cancel_rx.changed() => {
                        if changed.is_err() || *cancel_rx.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        let Some(now) = scheduler_time_or_failure(
                            now_timestamp(),
                            &after_cycle,
                        ).await else {
                            continue;
                        };
                        self.run_scheduler_cycle_at(now, &after_cycle).await;
                    }
                }
            }
        });
        QuotaSchedulerHandle {
            cancel_tx,
            task: Some(task),
        }
    }
}

async fn scheduler_time_or_failure(
    now: Result<i64, AppError>,
    after_cycle: &QuotaCycleCallback,
) -> Option<i64> {
    match now {
        Ok(now) => Some(now),
        Err(_) => {
            log::warn!("quota scheduler clock failed");
            after_cycle(QuotaSchedulerOutcome::Failed).await;
            None
        }
    }
}

fn quota_status_view(snapshot: &QuotaSnapshot) -> QuotaStatusView {
    QuotaStatusView::from_snapshot(snapshot)
}

pub struct QuotaSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl QuotaSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for QuotaSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn quota_interval_seconds(configured: Option<u64>) -> Result<Option<u64>, AppError> {
    match configured.unwrap_or(DEFAULT_QUOTA_INTERVAL_SECONDS) {
        0 => Ok(None),
        1..=59 => Err(AppError::Message(
            "quota interval must be 0 or at least 60 seconds".to_string(),
        )),
        interval => Ok(Some(interval)),
    }
}

fn quota_refresh_delay_seconds(state: &QuotaFetchState, interval: u64) -> u64 {
    if state.consecutive_failures == 0 {
        return interval;
    }
    let index = state
        .consecutive_failures
        .saturating_sub(1)
        .min((FAILURE_RETRY_DELAYS_SECONDS.len() - 1) as u32) as usize;
    FAILURE_RETRY_DELAYS_SECONDS[index]
}

pub fn normalize_subscription_quota(
    quota: &SubscriptionQuota,
    manual_resets_remaining: Option<i64>,
) -> Result<NormalizedQuota, AppError> {
    if !quota.success {
        return Err(AppError::Message(
            quota
                .error
                .clone()
                .or_else(|| quota.credential_message.clone())
                .unwrap_or_else(|| "quota collection failed".to_string()),
        ));
    }
    let tier = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| quota.tiers.iter().find(|tier| tier.name == *name))
    };
    let five_hour = tier(&[TIER_FIVE_HOUR]);
    let seven_day = tier(&[TIER_SEVEN_DAY, TIER_WEEKLY_LIMIT]);
    Ok(NormalizedQuota {
        five_hour_utilization_percent: five_hour
            .map(|tier| decimal_percent(tier.utilization))
            .transpose()?,
        five_hour_resets_at: five_hour.and_then(|tier| tier.resets_at.clone()),
        seven_day_utilization_percent: seven_day
            .map(|tier| decimal_percent(tier.utilization))
            .transpose()?,
        seven_day_resets_at: seven_day.and_then(|tier| tier.resets_at.clone()),
        manual_resets_remaining: quota
            .manual_reset_credits
            .as_ref()
            .map(|credits| credits.available_count)
            .or(manual_resets_remaining),
        raw_payload: serde_json::to_value(quota)
            .map_err(|error| AppError::Config(format!("serialize quota payload: {error}")))?,
    })
}

fn decimal_percent(value: f64) -> Result<String, AppError> {
    if !value.is_finite() || value < 0.0 {
        return Err(AppError::Message(
            "quota utilization must be a non-negative finite number".to_string(),
        ));
    }
    Decimal::from_str(&value.to_string())
        .map(|value| value.normalize().to_string())
        .map_err(|error| AppError::Message(format!("invalid quota utilization: {error}")))
}

fn manual_resets_from_config(config: &Value) -> Option<i64> {
    config
        .get("manual_resets_remaining")
        .or_else(|| config.get("manualResetsRemaining"))
        .and_then(Value::as_i64)
}

fn validate_refresh_provider(provider: &UsageProviderStored) -> Result<(), AppError> {
    if !provider.enabled {
        return Err(AppError::Message("quota provider is disabled".to_string()));
    }
    if provider.billing_kind != BillingKind::Subscription {
        return Err(AppError::Message(
            "quota provider must be subscription".to_string(),
        ));
    }
    Ok(())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Message(format!("system clock before unix epoch: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::codex_oauth_auth::CodexOAuthManager;
    use crate::database::Database;
    use crate::services::subscription::{
        CredentialStatus, ManualResetCredit, ManualResetCredits, QuotaTier, SubscriptionQuota,
        TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    };
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use crate::usage::system_providers::{CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID};
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::RwLock;

    fn successful_quota(tool: &str) -> SubscriptionQuota {
        SubscriptionQuota {
            tool: tool.to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![
                QuotaTier {
                    name: TIER_FIVE_HOUR.to_string(),
                    utilization: 12.5,
                    resets_at: Some("2026-07-11T05:00:00Z".to_string()),
                    used_value_usd: None,
                    max_value_usd: None,
                },
                QuotaTier {
                    name: TIER_SEVEN_DAY.to_string(),
                    utilization: 42.0,
                    resets_at: Some("2026-07-18T00:00:00Z".to_string()),
                    used_value_usd: None,
                    max_value_usd: None,
                },
            ],
            manual_reset_credits: None,
            extra_usage: None,
            error: None,
            queried_at: Some(100_000),
        }
    }

    fn provider(id: &str, billing_kind: BillingKind, enabled: bool) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: Some("fake".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: Some(json!({"manualResetsRemaining": 2})),
            enabled,
        }
    }

    fn isolated_quota_test_db() -> Arc<Database> {
        let db = Arc::new(Database::memory().unwrap());
        db.set_usage_provider_enabled(CHATGPT_SUBSCRIPTION_ID, false)
            .unwrap();
        db.set_usage_provider_enabled(CLAUDE_SUBSCRIPTION_ID, false)
            .unwrap();
        db
    }

    struct FakeCollector {
        calls: AtomicUsize,
        responses: Mutex<VecDeque<Result<SubscriptionQuota, String>>>,
    }

    impl FakeCollector {
        fn new(responses: Vec<Result<SubscriptionQuota, String>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                responses: Mutex::new(responses.into()),
            }
        }
    }

    impl QuotaCollector for FakeCollector {
        fn source(&self) -> &'static str {
            "fake"
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a UsageProviderStored,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self
                .responses
                .lock()
                .expect("fake responses")
                .pop_front()
                .expect("fake response");
            Box::pin(async move { result })
        }
    }

    #[derive(Default)]
    struct YieldingCollector {
        calls: AtomicUsize,
    }

    impl QuotaCollector for YieldingCollector {
        fn source(&self) -> &'static str {
            "fake"
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a UsageProviderStored,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok(successful_quota("claude"))
            })
        }
    }

    #[test]
    fn normalizes_documented_windows_and_optional_manual_resets() {
        let quota = successful_quota("claude");

        let normalized = normalize_subscription_quota(&quota, Some(3)).unwrap();
        assert_eq!(
            normalized.five_hour_utilization_percent.as_deref(),
            Some("12.5")
        );
        assert_eq!(
            normalized.seven_day_utilization_percent.as_deref(),
            Some("42")
        );
        assert_eq!(normalized.manual_resets_remaining, Some(3));
    }

    #[test]
    fn codex_reported_manual_reset_count_overrides_legacy_config_fallback() {
        let mut quota = successful_quota("codex");
        quota.manual_reset_credits = Some(ManualResetCredits {
            available_count: 3,
            credits: vec![ManualResetCredit {
                id: "reset-1".to_string(),
                reset_type: Some("codexRateLimits".to_string()),
                status: Some("available".to_string()),
                granted_at: None,
                expires_at: "2026-07-27T00:00:00+00:00".to_string(),
                title: Some("Full reset".to_string()),
                description: None,
            }],
        });

        let normalized = normalize_subscription_quota(&quota, Some(1)).unwrap();

        assert_eq!(normalized.manual_resets_remaining, Some(3));
        assert_eq!(
            normalized.raw_payload["manualResetCredits"]["credits"][0]["expiresAt"],
            json!("2026-07-27T00:00:00+00:00")
        );
    }

    #[test]
    fn interval_contract_defaults_to_five_minutes_and_zero_disables() {
        assert_eq!(quota_interval_seconds(None).unwrap(), Some(300));
        assert_eq!(quota_interval_seconds(Some(0)).unwrap(), None);
        assert!(quota_interval_seconds(Some(59)).is_err());
        assert_eq!(quota_interval_seconds(Some(60)).unwrap(), Some(60));
    }

    #[tokio::test]
    async fn production_registers_local_subscription_collectors() {
        let data_dir = std::env::temp_dir().join(format!(
            "llm-usage-bar-empty-codex-oauth-{}",
            Uuid::new_v4()
        ));
        let manager = Arc::new(RwLock::new(CodexOAuthManager::new(data_dir)));
        assert_eq!(Arc::strong_count(&manager), 1);

        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let service = QuotaService::production(db.clone(), manager.clone());
        let mut sources = service
            .collectors
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        sources.sort_unstable();

        assert_eq!(
            sources,
            [
                "claude",
                "claude_local",
                "codex",
                "codex_oauth",
                "coding_plan"
            ]
        );
        assert!(!service.collectors.contains_key("claude_oauth"));
        assert_eq!(Arc::strong_count(&manager), 2);

        let provider = db
            .get_usage_provider(crate::usage::system_providers::CHATGPT_SUBSCRIPTION_ID)
            .unwrap()
            .unwrap();
        let quota = service.collectors[MANAGED_CODEX_QUOTA_SOURCE]
            .collect(&provider)
            .await
            .unwrap();
        assert!(!quota.success);
        assert!(matches!(
            quota.credential_status,
            CredentialStatus::NotFound
        ));
    }

    #[test]
    fn claude_codex_and_coding_plan_share_window_normalization() {
        for tool in ["claude", "codex", "coding_plan"] {
            let mut quota = successful_quota(tool);
            if tool == "coding_plan" {
                quota.tiers[1].name = TIER_WEEKLY_LIMIT.to_string();
            }
            let normalized = normalize_subscription_quota(&quota, None).unwrap();
            assert_eq!(
                normalized.five_hour_utilization_percent.as_deref(),
                Some("12.5")
            );
            assert_eq!(
                normalized.seven_day_utilization_percent.as_deref(),
                Some("42")
            );
            assert_eq!(normalized.manual_resets_remaining, None);
        }
    }

    #[test]
    fn scheduler_can_start_without_an_ambient_tokio_runtime() {
        let service = Arc::new(QuotaService::new(isolated_quota_test_db()));

        let scheduler = service.start_scheduler(Arc::new(|_| Box::pin(async {})));

        drop(scheduler);
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_zero_attempt_success_once() {
        let service = QuotaService::new(isolated_quota_test_db());
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        assert_eq!(
            *outcomes.lock().expect("captured outcomes"),
            vec![QuotaSchedulerOutcome::Completed {
                attempted: 0,
                had_errors: false,
            }]
        );
    }

    #[tokio::test]
    async fn scheduler_cycle_waits_for_async_callback_completion() {
        let service = Arc::new(QuotaService::new(isolated_quota_test_db()));
        let callback_started = Arc::new(tokio::sync::Notify::new());
        let release_callback = Arc::new(tokio::sync::Notify::new());
        let callback_finished = Arc::new(AtomicUsize::new(0));
        let after_cycle: QuotaCycleCallback = {
            let callback_started = callback_started.clone();
            let release_callback = release_callback.clone();
            let callback_finished = callback_finished.clone();
            Arc::new(move |_| {
                let callback_started = callback_started.clone();
                let release_callback = release_callback.clone();
                let callback_finished = callback_finished.clone();
                Box::pin(async move {
                    callback_started.notify_one();
                    release_callback.notified().await;
                    callback_finished.fetch_add(1, Ordering::SeqCst);
                })
            })
        };

        let cycle = tokio::spawn({
            let service = service.clone();
            async move {
                service.run_scheduler_cycle_at(100, &after_cycle).await;
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            callback_started.notified(),
        )
        .await
        .expect("callback should start");
        assert_eq!(callback_finished.load(Ordering::SeqCst), 0);

        release_callback.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), cycle)
            .await
            .expect("scheduler cycle should finish after its callback")
            .expect("scheduler cycle task should not panic");
        assert_eq!(callback_finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_provider_errors_without_details() {
        const SECRET_SENTINEL: &str = "sk-secret-scheduler-provider-error";
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![Err(SECRET_SENTINEL.to_string())]));
        let service = QuotaService::with_collectors(db, vec![collector]);
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(
            outcomes.as_slice(),
            [QuotaSchedulerOutcome::Completed {
                attempted: 1,
                had_errors: true,
            }]
        );
        assert!(!format!("{outcomes:?}").contains(SECRET_SENTINEL));
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_top_level_failure_once() {
        let db = isolated_quota_test_db();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "ALTER TABLE usage_providers RENAME TO unavailable_usage_providers",
                [],
            )
            .unwrap();
        let service = QuotaService::with_collectors(db, vec![]);
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(outcomes.as_slice(), [QuotaSchedulerOutcome::Failed]);
        assert!(!format!("{outcomes:?}").contains("usage_providers"));
    }

    #[tokio::test]
    async fn scheduler_clock_failure_reports_sanitized_failure_once() {
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        let now = scheduler_time_or_failure(
            Err(AppError::Message(
                "system clock before unix epoch: secret-clock-details".to_string(),
            )),
            &after_cycle,
        )
        .await;

        assert_eq!(now, None);
        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(outcomes.as_slice(), [QuotaSchedulerOutcome::Failed]);
        assert!(!format!("{outcomes:?}").contains("secret-clock-details"));
    }

    #[tokio::test]
    async fn failures_follow_backoff_and_only_expire_after_ten_minute_retry() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
        ]));
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert!(first.errors.is_empty());
        let first_snapshot = db.latest_quota_snapshot("sub").unwrap().unwrap();
        assert_eq!(first_snapshot.fetched_at, 100);
        assert_eq!(first_snapshot.manual_resets_remaining, Some(2));

        assert_eq!(service.refresh_due_at(399).await.unwrap().attempted, 0);
        let failed = service.refresh_due_at(400).await.unwrap();
        assert_eq!(failed.attempted, 1);
        assert_eq!(failed.errors.len(), 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            db.latest_quota_snapshot("sub").unwrap().unwrap(),
            first_snapshot
        );
        let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
        assert_eq!(state.last_success_at, Some(100));
        assert_eq!(state.last_attempt_at, Some(400));
        assert_eq!(state.consecutive_failures, 1);
        assert!(!state.stale);

        for (not_due_at, due_at, expected_failures, expected_stale) in [
            (429, 430, 2, false),
            (489, 490, 3, false),
            (789, 790, 4, false),
            (1_389, 1_390, 5, true),
            (3_189, 3_190, 6, true),
        ] {
            assert_eq!(
                service.refresh_due_at(not_due_at).await.unwrap().attempted,
                0
            );
            assert_eq!(service.refresh_due_at(due_at).await.unwrap().attempted, 1);
            let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
            assert_eq!(state.consecutive_failures, expected_failures);
            assert_eq!(state.stale, expected_stale);
        }
        assert_eq!(collector.calls.load(Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn custom_interval_only_changes_healthy_refresh_cadence() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_interval_seconds = Some(480);
        db.save_usage_provider(&input).unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
        ]));
        let service = QuotaService::with_collectors(db, vec![collector]);

        assert_eq!(service.refresh_due_at(100).await.unwrap().attempted, 1);
        assert_eq!(service.refresh_due_at(579).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(580).await.unwrap().attempted, 1);
        assert_eq!(service.refresh_due_at(609).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(610).await.unwrap().attempted, 1);
    }

    #[tokio::test]
    async fn manual_refresh_collects_once_and_rejects_metered_or_disabled() {
        let db = isolated_quota_test_db();
        for input in [
            provider("sub", BillingKind::Subscription, true),
            provider("metered", BillingKind::Metered, true),
            provider("disabled", BillingKind::Subscription, false),
        ] {
            db.save_usage_provider(&input).unwrap();
        }
        let collector = Arc::new(FakeCollector::new(vec![Ok(successful_quota("claude"))]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        let refreshed = service.refresh_provider("sub").await.unwrap();
        let public_json = serde_json::to_value(&refreshed).unwrap();
        assert!(public_json["snapshot"].get("rawPayload").is_none());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert!(service.refresh_provider("metered").await.is_err());
        assert!(service.refresh_provider("disabled").await.is_err());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn zero_interval_disables_scheduled_collection() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_interval_seconds = Some(0);
        db.save_usage_provider(&input).unwrap();
        let collector = Arc::new(FakeCollector::new(vec![]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        assert_eq!(service.refresh_due_at(1_000).await.unwrap().attempted, 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configuration_failures_enter_the_retry_schedule() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_source = Some("unsupported".to_string());
        db.save_usage_provider(&input).unwrap();
        let service = QuotaService::with_collectors(db.clone(), vec![]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert_eq!(first.errors.len(), 1);
        assert_eq!(
            db.get_quota_fetch_state("sub")
                .unwrap()
                .unwrap()
                .last_attempt_at,
            Some(100)
        );
        assert_eq!(service.refresh_due_at(101).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(129).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(130).await.unwrap().attempted, 1);
    }

    #[tokio::test]
    async fn normalization_failures_enter_the_retry_schedule() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let mut invalid = successful_quota("claude");
        invalid.tiers[0].utilization = f64::NAN;
        let collector = Arc::new(FakeCollector::new(vec![Ok(invalid.clone()), Ok(invalid)]));
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert_eq!(first.errors.len(), 1);
        let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
        assert_eq!(state.last_attempt_at, Some(100));
        assert!(state
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("non-negative finite"));
        assert_eq!(service.refresh_due_at(129).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(130).await.unwrap().attempted, 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn machine_local_quota_source_cannot_be_attributed_to_two_providers() {
        for source in ["claude", "claude_local", "codex", "codex_oauth"] {
            let db = isolated_quota_test_db();
            for id in ["first", "second"] {
                let mut input = provider(id, BillingKind::Subscription, true);
                input.quota_source = Some(source.to_string());
                db.save_usage_provider(&input).unwrap();
            }
            let service = QuotaService::with_collectors(db.clone(), vec![]);

            let cycle = service.refresh_due_at(500).await.unwrap();
            assert_eq!(cycle.attempted, 2, "{source}");
            assert_eq!(cycle.errors.len(), 2, "{source}");
            for id in ["first", "second"] {
                let state = db.get_quota_fetch_state(id).unwrap().unwrap();
                assert_eq!(state.last_attempt_at, Some(500), "{source}/{id}");
                assert!(
                    state
                        .last_error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("exactly one enabled provider"),
                    "{source}/{id}: {:?}",
                    state.last_error
                );
            }
        }
    }

    #[tokio::test]
    async fn concurrent_manual_refreshes_share_one_provider_snapshot() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(YieldingCollector::default());
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let (first, second) = tokio::join!(
            service.refresh_provider("sub"),
            service.refresh_provider("sub")
        );
        let first = first.unwrap();
        let second = second.unwrap();

        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM quota_snapshots WHERE provider_id = 'sub'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn manual_and_scheduler_refresh_share_the_same_provider_flight() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(YieldingCollector::default());
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let (manual, scheduler) =
            tokio::join!(service.refresh_provider("sub"), service.refresh_due_at(100));

        manual.unwrap();
        assert!(scheduler.unwrap().errors.is_empty());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM quota_snapshots WHERE provider_id = 'sub'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
    }
}
