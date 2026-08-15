use crate::error::AppError;
use crate::model::{BindingCredentialStatus, ProviderApiKeyView, UsageProviderView};
use crate::providers::SystemProviderConnectionService;
use crate::secrets::BindingCredentialService;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

const TICK_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const FAILURE_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(15 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(60 * 60),
];

type ProviderListFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<UsageProviderView>, AppError>> + Send + 'a>>;
type RefreshFuture<'a> = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + 'a>>;

trait SchedulerDependencies: Send + Sync {
    fn list_providers(&self) -> ProviderListFuture<'_>;
    fn refresh_key_usage<'a>(
        &'a self,
        key_id: &'a str,
        credential_version: u64,
    ) -> RefreshFuture<'a>;
    fn now(&self) -> Result<i64, AppError>;
    fn is_lightweight_mode(&self) -> bool;
    fn notify_dashboard_invalidated(&self);
}

struct ProductionDependencies {
    credentials: Arc<BindingCredentialService>,
    connection: Arc<SystemProviderConnectionService>,
}

impl SchedulerDependencies for ProductionDependencies {
    fn list_providers(&self) -> ProviderListFuture<'_> {
        Box::pin(self.credentials.list_usage_providers())
    }

    fn refresh_key_usage<'a>(
        &'a self,
        key_id: &'a str,
        credential_version: u64,
    ) -> RefreshFuture<'a> {
        Box::pin(async move {
            self.connection
                .refresh_key_usage(key_id, credential_version)
                .await
                .map(|_| ())
        })
    }

    fn now(&self) -> Result<i64, AppError> {
        unix_timestamp()
    }

    fn is_lightweight_mode(&self) -> bool {
        crate::lightweight::is_lightweight_mode()
    }

    fn notify_dashboard_invalidated(&self) {
        crate::usage_events::notify_dashboard_invalidated();
    }
}

#[derive(Debug, Clone, Copy)]
struct FailureState {
    consecutive_failures: usize,
    failed_at: i64,
}

#[derive(Debug, Default)]
struct KeyFailureBackoff {
    keys: HashMap<String, FailureState>,
}

impl KeyFailureBackoff {
    fn allows_retry(&self, key_id: &str, now: i64) -> bool {
        self.keys.get(key_id).is_none_or(|state| {
            now.saturating_sub(state.failed_at)
                >= failure_retry_delay(state.consecutive_failures).as_secs() as i64
        })
    }

    fn record_failure(&mut self, key_id: &str, now: i64) {
        let previous = self
            .keys
            .get(key_id)
            .map_or(0, |state| state.consecutive_failures);
        self.keys.insert(
            key_id.to_string(),
            FailureState {
                consecutive_failures: previous.saturating_add(1),
                failed_at: now,
            },
        );
    }

    fn reset(&mut self, key_id: &str) {
        self.keys.remove(key_id);
    }

    fn retain(&mut self, eligible_key_ids: &HashSet<&str>) {
        self.keys
            .retain(|key_id, _| eligible_key_ids.contains(key_id.as_str()));
    }
}

fn failure_retry_delay(consecutive_failures: usize) -> Duration {
    let index = consecutive_failures
        .saturating_sub(1)
        .min(FAILURE_RETRY_DELAYS.len().saturating_sub(1));
    FAILURE_RETRY_DELAYS[index]
}

fn is_provider_eligible(provider: &UsageProviderView) -> bool {
    provider.supports_key_usage && provider.enabled
}

fn is_key_eligible(key: &ProviderApiKeyView) -> bool {
    key.credential_status == BindingCredentialStatus::Configured
}

fn is_due(key: &ProviderApiKeyView, now: i64) -> bool {
    key.key_usage.as_ref().is_none_or(|snapshot| {
        snapshot.credential_version != key.credential_version
            || now.saturating_sub(snapshot.fetched_at) >= REFRESH_INTERVAL.as_secs() as i64
    })
}

fn is_credential_conflict(error: &AppError) -> bool {
    matches!(error, AppError::Message(code) if code == "credential_conflict")
}

pub struct ProviderKeyUsageSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl ProviderKeyUsageSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for ProviderKeyUsageSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn start_scheduler(
    credentials: Arc<BindingCredentialService>,
    connection: Arc<SystemProviderConnectionService>,
) -> ProviderKeyUsageSchedulerHandle {
    start_scheduler_with_dependencies(
        Arc::new(ProductionDependencies {
            credentials,
            connection,
        }),
        TICK_INTERVAL,
    )
}

fn start_scheduler_with_dependencies(
    dependencies: Arc<dyn SchedulerDependencies>,
    tick_interval: Duration,
) -> ProviderKeyUsageSchedulerHandle {
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let task = tauri::async_runtime::spawn(async move {
        let mut backoff = KeyFailureBackoff::default();

        if !refresh_with_backoff_or_cancel(&*dependencies, &mut backoff, &mut cancel_rx).await {
            return;
        }

        loop {
            if !wait_or_cancel(tick_interval, &mut cancel_rx).await {
                return;
            }
            if !refresh_with_backoff_or_cancel(&*dependencies, &mut backoff, &mut cancel_rx).await {
                return;
            }
        }
    });
    ProviderKeyUsageSchedulerHandle {
        cancel_tx,
        task: Some(task),
    }
}

async fn refresh_with_backoff_or_cancel(
    dependencies: &dyn SchedulerDependencies,
    backoff: &mut KeyFailureBackoff,
    cancel_rx: &mut watch::Receiver<bool>,
) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = refresh_with_backoff(dependencies, backoff) => true,
    }
}

async fn refresh_with_backoff(
    dependencies: &dyn SchedulerDependencies,
    backoff: &mut KeyFailureBackoff,
) {
    if dependencies.is_lightweight_mode() {
        return;
    }

    let now = match dependencies.now() {
        Ok(now) => now,
        Err(_) => {
            log::warn!("provider key usage scheduler could not read the system clock");
            return;
        }
    };
    let providers = match dependencies.list_providers().await {
        Ok(providers) => providers,
        Err(_) => {
            log::warn!("provider key usage scheduler could not list providers");
            return;
        }
    };
    let eligible_key_ids = providers
        .iter()
        .filter(|provider| is_provider_eligible(provider))
        .flat_map(|provider| provider.api_keys.iter())
        .filter(|key| is_key_eligible(key))
        .map(|key| key.id.as_str())
        .collect::<HashSet<_>>();
    backoff.retain(&eligible_key_ids);

    for provider in providers.into_iter().filter(is_provider_eligible) {
        for key in provider.api_keys.into_iter().filter(is_key_eligible) {
            if !is_due(&key, now) || !backoff.allows_retry(&key.id, now) {
                continue;
            }

            match dependencies
                .refresh_key_usage(&key.id, key.credential_version)
                .await
            {
                Ok(()) => {
                    backoff.reset(&key.id);
                    log::debug!(
                        "provider key usage refreshed for provider {} key {}",
                        provider.id,
                        key.id
                    );
                    dependencies.notify_dashboard_invalidated();
                }
                Err(error) if is_credential_conflict(&error) => {
                    backoff.reset(&key.id);
                    log::debug!(
                        "provider key usage refresh will re-read credentials for provider {} key {}",
                        provider.id,
                        key.id
                    );
                }
                Err(_) => {
                    let failed_at = dependencies.now().unwrap_or(now);
                    backoff.record_failure(&key.id, failed_at);
                    log::warn!(
                        "provider key usage refresh failed for provider {} key {}",
                        provider.id,
                        key.id
                    );
                }
            }
        }
    }
}

async fn wait_or_cancel(duration: Duration, cancel_rx: &mut watch::Receiver<bool>) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = tokio::time::sleep(duration) => true,
    }
}

fn unix_timestamp() -> Result<i64, AppError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Message("system clock before unix epoch".to_string()))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| AppError::Message("system timestamp exceeds i64".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BillingKind, SystemProviderKeyUsageView};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn key(id: &str) -> ProviderApiKeyView {
        ProviderApiKeyView {
            id: id.to_string(),
            provider_id: "provider-a".to_string(),
            label: id.to_string(),
            credential_status: BindingCredentialStatus::Configured,
            credential_version: 2,
            can_clear_credential: true,
            last_connection_test_at: None,
            last_connection_test_status: None,
            last_connection_test_error_code: None,
            sort_order: 0,
            key_usage: None,
        }
    }

    fn provider(id: &str) -> UsageProviderView {
        let mut api_key = key(&format!("{id}-key-a"));
        api_key.provider_id = id.to_string();
        UsageProviderView {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: id.to_string(),
            token_sources: Vec::new(),
            session_source_bindings: Vec::new(),
            bindings: Vec::new(),
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            enabled: true,
            needs_review: false,
            created_at: 0,
            updated_at: 0,
            route_base_url: None,
            has_route_credentials: false,
            system_preset_key: Some(id.to_string()),
            system_auth_kind: None,
            canonical_endpoint: None,
            compatible_agent_module_ids: Vec::new(),
            api_keys: vec![api_key],
            daily_budget_usd: None,
            supports_key_usage: true,
            key_usage_total: None,
        }
    }

    fn snapshot(fetched_at: i64, credential_version: u64) -> SystemProviderKeyUsageView {
        SystemProviderKeyUsageView {
            usage_total_usd: None,
            usage_daily_usd: None,
            usage_weekly_usd: None,
            usage_monthly_usd: None,
            limit_usd: None,
            limit_remaining_usd: None,
            is_free_tier: None,
            fetched_at,
            credential_version,
            stale: false,
        }
    }

    #[test]
    fn due_uses_snapshot_timestamp_and_credential_version() {
        let now = 10_000;
        let mut candidate = key("key-a");
        assert!(is_due(&candidate, now));

        candidate.key_usage = Some(snapshot(now - REFRESH_INTERVAL.as_secs() as i64, 2));
        assert!(is_due(&candidate, now));

        candidate.key_usage = Some(snapshot(now - REFRESH_INTERVAL.as_secs() as i64 + 1, 2));
        assert!(!is_due(&candidate, now));

        candidate.key_usage = Some(snapshot(now, 1));
        assert!(is_due(&candidate, now));
    }

    #[test]
    fn eligibility_requires_support_enabled_and_configured_credential() {
        let mut candidate = provider("provider-a");
        assert!(is_provider_eligible(&candidate));
        assert!(is_key_eligible(&candidate.api_keys[0]));

        candidate.supports_key_usage = false;
        assert!(!is_provider_eligible(&candidate));

        candidate.supports_key_usage = true;
        candidate.enabled = false;
        assert!(!is_provider_eligible(&candidate));

        candidate.enabled = true;
        candidate.api_keys[0].credential_status = BindingCredentialStatus::Missing;
        assert!(is_provider_eligible(&candidate));
        assert!(!is_key_eligible(&candidate.api_keys[0]));
    }

    #[test]
    fn failure_backoff_grows_caps_resets_and_is_per_key() {
        let mut backoff = KeyFailureBackoff::default();

        backoff.record_failure("key-a", 100);
        assert_eq!(failure_retry_delay(1), Duration::from_secs(15 * 60));
        assert!(!backoff.allows_retry("key-a", 999));
        assert!(backoff.allows_retry("key-a", 1_000));
        assert!(backoff.allows_retry("key-b", 100));

        backoff.record_failure("key-a", 1_000);
        assert_eq!(failure_retry_delay(2), Duration::from_secs(30 * 60));
        backoff.record_failure("key-a", 2_800);
        assert_eq!(failure_retry_delay(3), Duration::from_secs(60 * 60));
        backoff.record_failure("key-a", 6_400);
        assert_eq!(failure_retry_delay(4), Duration::from_secs(60 * 60));

        backoff.reset("key-a");
        assert!(backoff.allows_retry("key-a", 100));
        backoff.record_failure("key-a", 100);
        assert_eq!(backoff.keys["key-a"].consecutive_failures, 1);
    }

    struct FakeDependencies {
        providers: Vec<UsageProviderView>,
        now: AtomicI64,
        lightweight: AtomicBool,
        list_calls: AtomicUsize,
        refresh_calls: Mutex<Vec<String>>,
        refresh_results: Mutex<VecDeque<Result<(), AppError>>>,
        notifications: AtomicUsize,
        pending_list: bool,
    }

    impl FakeDependencies {
        fn new(providers: Vec<UsageProviderView>) -> Self {
            Self {
                providers,
                now: AtomicI64::new(10_000),
                lightweight: AtomicBool::new(false),
                list_calls: AtomicUsize::new(0),
                refresh_calls: Mutex::new(Vec::new()),
                refresh_results: Mutex::new(VecDeque::new()),
                notifications: AtomicUsize::new(0),
                pending_list: false,
            }
        }
    }

    impl SchedulerDependencies for FakeDependencies {
        fn list_providers(&self) -> ProviderListFuture<'_> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            if self.pending_list {
                Box::pin(std::future::pending())
            } else {
                Box::pin(async { Ok(self.providers.clone()) })
            }
        }

        fn refresh_key_usage<'a>(
            &'a self,
            key_id: &'a str,
            _credential_version: u64,
        ) -> RefreshFuture<'a> {
            self.refresh_calls.lock().unwrap().push(key_id.to_string());
            let result = self
                .refresh_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(()));
            Box::pin(async move { result })
        }

        fn now(&self) -> Result<i64, AppError> {
            Ok(self.now.load(Ordering::SeqCst))
        }

        fn is_lightweight_mode(&self) -> bool {
            self.lightweight.load(Ordering::SeqCst)
        }

        fn notify_dashboard_invalidated(&self) {
            self.notifications.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn two_keys_under_one_provider_refresh_independently() {
        let mut candidate = provider("provider-a");
        let mut second_key = key("provider-a-key-b");
        second_key.provider_id = candidate.id.clone();
        second_key.sort_order = 1;
        candidate.api_keys.push(second_key);
        let dependencies = FakeDependencies::new(vec![candidate]);
        dependencies.refresh_results.lock().unwrap().extend([
            Err(AppError::Message("connection_failed".to_string())),
            Ok(()),
        ]);
        let mut backoff = KeyFailureBackoff::default();

        refresh_with_backoff(&dependencies, &mut backoff).await;

        assert_eq!(
            *dependencies.refresh_calls.lock().unwrap(),
            vec!["provider-a-key-a", "provider-a-key-b"]
        );
        assert!(backoff.keys.contains_key("provider-a-key-a"));
        assert!(!backoff.keys.contains_key("provider-a-key-b"));
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 1);

        refresh_with_backoff(&dependencies, &mut backoff).await;
        assert_eq!(
            *dependencies.refresh_calls.lock().unwrap(),
            vec!["provider-a-key-a", "provider-a-key-b", "provider-a-key-b"]
        );
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn scheduler_success_resets_a_keys_failure_count() {
        let dependencies = FakeDependencies::new(vec![provider("provider-a")]);
        dependencies.refresh_results.lock().unwrap().extend([
            Err(AppError::Message("connection_failed".to_string())),
            Ok(()),
            Err(AppError::Message("connection_failed".to_string())),
        ]);
        let mut backoff = KeyFailureBackoff::default();

        refresh_with_backoff(&dependencies, &mut backoff).await;
        assert_eq!(backoff.keys["provider-a-key-a"].consecutive_failures, 1);

        dependencies.now.store(10_900, Ordering::SeqCst);
        refresh_with_backoff(&dependencies, &mut backoff).await;
        assert!(backoff.keys.is_empty());

        refresh_with_backoff(&dependencies, &mut backoff).await;
        assert_eq!(backoff.keys["provider-a-key-a"].consecutive_failures, 1);
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn credential_conflict_is_retried_without_backoff() {
        let dependencies = FakeDependencies::new(vec![provider("provider-a")]);
        dependencies.refresh_results.lock().unwrap().extend([
            Err(AppError::Message("credential_conflict".to_string())),
            Ok(()),
        ]);
        let mut backoff = KeyFailureBackoff::default();

        refresh_with_backoff(&dependencies, &mut backoff).await;
        assert!(backoff.keys.is_empty());
        refresh_with_backoff(&dependencies, &mut backoff).await;

        assert_eq!(dependencies.refresh_calls.lock().unwrap().len(), 2);
        assert!(backoff.keys.is_empty());
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn lightweight_tick_does_no_provider_or_refresh_work() {
        let dependencies = FakeDependencies::new(vec![provider("provider-a")]);
        dependencies.lightweight.store(true, Ordering::SeqCst);
        let mut backoff = KeyFailureBackoff::default();

        refresh_with_backoff(&dependencies, &mut backoff).await;

        assert_eq!(dependencies.list_calls.load(Ordering::SeqCst), 0);
        assert!(dependencies.refresh_calls.lock().unwrap().is_empty());
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn scheduler_stop_cancels_a_started_tick_promptly() {
        let mut dependencies = FakeDependencies::new(Vec::new());
        dependencies.pending_list = true;
        let handle =
            start_scheduler_with_dependencies(Arc::new(dependencies), Duration::from_secs(60 * 60));
        tokio::task::yield_now().await;

        tokio::time::timeout(Duration::from_millis(250), handle.stop())
            .await
            .expect("scheduler stop should not wait for the pending provider listing");
    }
}
