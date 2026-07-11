use crate::database::Database;
use crate::error::AppError;
use crate::services::coding_plan::get_coding_plan_quota;
use crate::services::subscription::{
    get_subscription_quota, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY, TIER_WEEKLY_LIMIT,
};
use crate::usage::domain::{
    BillingKind, QuotaFetchState, QuotaSnapshot, QuotaStatusView, UsageProviderStored,
};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub const DEFAULT_QUOTA_INTERVAL_SECONDS: u64 = 300;
const SCHEDULER_TICK_SECONDS: u64 = 60;

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

#[derive(Clone)]
pub struct QuotaService {
    db: Arc<Database>,
    collectors: Arc<HashMap<String, Arc<dyn QuotaCollector>>>,
}

impl QuotaService {
    pub fn new(db: Arc<Database>) -> Self {
        Self::with_collectors(
            db,
            vec![
                Arc::new(SubscriptionQuotaCollector { source: "claude" }),
                Arc::new(SubscriptionQuotaCollector { source: "codex" }),
                Arc::new(CodingPlanQuotaCollector),
            ],
        )
    }

    pub fn with_collectors(db: Arc<Database>, collectors: Vec<Arc<dyn QuotaCollector>>) -> Self {
        let collectors = collectors
            .into_iter()
            .map(|collector| (collector.source().to_string(), collector))
            .collect();
        Self {
            db,
            collectors: Arc::new(collectors),
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

        let quota = match collector.collect(&provider).await {
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
        if !matches!(quota_source, "claude" | "codex") {
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

    /// Runs one deterministic scheduling pass. Failed collections have their
    /// attempt persisted, so subsequent passes wait for the full configured
    /// interval instead of entering a retry loop.
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
            let due = state
                .and_then(|state| state.last_attempt_at)
                .is_none_or(|last_attempt| now.saturating_sub(last_attempt) >= interval as i64);
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

    pub fn start_scheduler(self: Arc<Self>) -> QuotaSchedulerHandle {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
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
                        let now = match now_timestamp() {
                            Ok(now) => now,
                            Err(error) => {
                                log::warn!("quota scheduler clock failed: {error}");
                                continue;
                            }
                        };
                        match self.refresh_due_at(now).await {
                            Ok(cycle) => {
                                for error in cycle.errors {
                                    log::warn!("quota scheduler refresh failed: {error}");
                                }
                            }
                            Err(error) => log::warn!("quota scheduler cycle failed: {error}"),
                        }
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

fn quota_status_view(snapshot: &QuotaSnapshot) -> QuotaStatusView {
    QuotaStatusView {
        snapshot_id: snapshot.snapshot_id.clone(),
        fetched_at: snapshot.fetched_at,
        five_hour_utilization_percent: snapshot.five_hour_utilization_percent.clone(),
        five_hour_resets_at: snapshot.five_hour_resets_at.clone(),
        seven_day_utilization_percent: snapshot.seven_day_utilization_percent.clone(),
        seven_day_resets_at: snapshot.seven_day_resets_at.clone(),
        manual_resets_remaining: snapshot.manual_resets_remaining,
    }
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
        manual_resets_remaining,
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
    use crate::database::Database;
    use crate::services::subscription::{
        CredentialStatus, QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    };
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

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
    fn interval_contract_defaults_to_five_minutes_and_zero_disables() {
        assert_eq!(quota_interval_seconds(None).unwrap(), Some(300));
        assert_eq!(quota_interval_seconds(Some(0)).unwrap(), None);
        assert!(quota_interval_seconds(Some(59)).is_err());
        assert_eq!(quota_interval_seconds(Some(60)).unwrap(), Some(60));
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

    #[tokio::test]
    async fn success_appends_and_failure_preserves_success_without_tight_retry() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
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
        assert!(state.stale);

        assert_eq!(service.refresh_due_at(401).await.unwrap().attempted, 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn manual_refresh_collects_once_and_rejects_metered_or_disabled() {
        let db = Arc::new(Database::memory().unwrap());
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
        let db = Arc::new(Database::memory().unwrap());
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_interval_seconds = Some(0);
        db.save_usage_provider(&input).unwrap();
        let collector = Arc::new(FakeCollector::new(vec![]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        assert_eq!(service.refresh_due_at(1_000).await.unwrap().attempted, 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configuration_failures_wait_for_the_full_interval() {
        let db = Arc::new(Database::memory().unwrap());
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
        assert_eq!(service.refresh_due_at(399).await.unwrap().attempted, 0);
    }

    #[tokio::test]
    async fn normalization_failures_wait_for_the_full_interval() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let mut invalid = successful_quota("claude");
        invalid.tiers[0].utilization = f64::NAN;
        let collector = Arc::new(FakeCollector::new(vec![Ok(invalid)]));
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
        assert_eq!(service.refresh_due_at(399).await.unwrap().attempted, 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn machine_local_quota_source_cannot_be_attributed_to_two_providers() {
        let db = Arc::new(Database::memory().unwrap());
        for id in ["first", "second"] {
            let mut input = provider(id, BillingKind::Subscription, true);
            input.quota_source = Some("claude".to_string());
            db.save_usage_provider(&input).unwrap();
        }
        let service = QuotaService::with_collectors(db.clone(), vec![]);

        let cycle = service.refresh_due_at(500).await.unwrap();
        assert_eq!(cycle.attempted, 2);
        assert_eq!(cycle.errors.len(), 2);
        for id in ["first", "second"] {
            let state = db.get_quota_fetch_state(id).unwrap().unwrap();
            assert_eq!(state.last_attempt_at, Some(500));
            assert!(state
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("exactly one enabled provider"));
        }
    }
}
