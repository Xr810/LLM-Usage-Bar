use crate::credentials::{unavailable_credential_store, BindingCredentialService, CredentialStore};
#[cfg(test)]
use crate::services::claude_cli_auth::ClaudeAuthCommandRunner;
use crate::services::{
    official_pricing::{
        start_scheduler as start_official_pricing_scheduler, OfficialPricingSchedulerHandle,
    },
    provider_key_usage_scheduler::{
        start_scheduler as start_provider_key_usage_scheduler, ProviderKeyUsageSchedulerHandle,
    },
    tray_usage::TrayUsageService,
    tray_usage_scheduler::{
        start_local_midnight_scheduler, TraySnapshotPublisher, TrayUsageSchedulerHandle,
    },
    ClaudeCliAuthService, SystemProviderConnectionService, UsageCache,
};
use crate::store::Database;
use crate::usage::quota::{QuotaCycleCallback, QuotaSchedulerHandle, QuotaService};
use crate::usage::session::SessionUsageService;
use std::sync::{Arc, Mutex};

/// 全局应用状态
pub struct AppState {
    pub db: Arc<Database>,
    pub credential_store: Arc<dyn CredentialStore>,
    pub binding_credential_service: Arc<BindingCredentialService>,
    pub claude_cli_auth_service: Arc<ClaudeCliAuthService>,
    pub system_provider_connection_service: Arc<SystemProviderConnectionService>,
    pub usage_cache: Arc<UsageCache>,
    pub quota_service: Arc<QuotaService>,
    pub session_usage_service: Arc<SessionUsageService>,
    pub tray_usage_service: Arc<TrayUsageService>,
    quota_scheduler: Mutex<Option<QuotaSchedulerHandle>>,
    midnight_scheduler: Mutex<Option<TrayUsageSchedulerHandle>>,
    official_pricing_scheduler: Mutex<Option<OfficialPricingSchedulerHandle>>,
    provider_key_usage_scheduler: Mutex<Option<ProviderKeyUsageSchedulerHandle>>,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        Self::new_with_credential_store(db, unavailable_credential_store())
    }

    pub fn new_with_credential_store(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        let quota_service = Arc::new(QuotaService::new(db.clone()));
        Self::new_with_services(
            db,
            credential_store,
            Arc::new(ClaudeCliAuthService::production()),
            quota_service,
        )
    }

    pub fn new_with_credential_store_and_quota_service(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        quota_service: Arc<QuotaService>,
    ) -> Self {
        Self::new_with_services(
            db,
            credential_store,
            Arc::new(ClaudeCliAuthService::production()),
            quota_service,
        )
    }

    #[cfg(test)]
    pub fn new_with_credential_store_and_claude_auth_runner(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        runner: Arc<dyn ClaudeAuthCommandRunner>,
    ) -> Self {
        let quota_service = Arc::new(QuotaService::new(db.clone()));
        Self::new_with_services(
            db,
            credential_store,
            Arc::new(ClaudeCliAuthService::new(runner)),
            quota_service,
        )
    }

    fn new_with_services(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        claude_cli_auth_service: Arc<ClaudeCliAuthService>,
        quota_service: Arc<QuotaService>,
    ) -> Self {
        let binding_credential_service = Arc::new(BindingCredentialService::new(
            db.clone(),
            credential_store.clone(),
        ));
        let system_provider_connection_service =
            Arc::new(SystemProviderConnectionService::production(
                db.clone(),
                binding_credential_service.clone(),
            ));
        let session_usage_service = Arc::new(SessionUsageService::new(db.clone()));
        let tray_usage_service = Arc::new(TrayUsageService::new(
            db.clone(),
            quota_service.clone(),
            session_usage_service.clone(),
        ));

        Self {
            db,
            credential_store,
            binding_credential_service,
            claude_cli_auth_service,
            system_provider_connection_service,
            usage_cache: Arc::new(UsageCache::new()),
            quota_service,
            session_usage_service,
            tray_usage_service,
            quota_scheduler: Mutex::new(None),
            midnight_scheduler: Mutex::new(None),
            official_pricing_scheduler: Mutex::new(None),
            provider_key_usage_scheduler: Mutex::new(None),
        }
    }

    pub fn start_quota_scheduler(&self, after_cycle: QuotaCycleCallback) -> bool {
        let Ok(mut scheduler) = self.quota_scheduler.lock() else {
            log::error!("quota scheduler lock is poisoned");
            return false;
        };
        if scheduler.is_some() {
            return false;
        }
        *scheduler = Some(self.quota_service.clone().start_scheduler(after_cycle));
        true
    }

    pub async fn stop_quota_scheduler(&self) {
        if let Some(scheduler) = self.take_quota_scheduler() {
            scheduler.stop().await;
        }
    }

    pub(crate) fn take_quota_scheduler(&self) -> Option<QuotaSchedulerHandle> {
        match self.quota_scheduler.lock() {
            Ok(mut scheduler) => scheduler.take(),
            Err(_) => {
                log::error!("quota scheduler lock is poisoned");
                None
            }
        }
    }

    pub fn start_midnight_scheduler(&self, publish: TraySnapshotPublisher) -> bool {
        let Ok(mut scheduler) = self.midnight_scheduler.lock() else {
            log::error!("tray usage midnight scheduler lock is poisoned");
            return false;
        };
        if scheduler.is_some() {
            return false;
        }
        *scheduler = Some(start_local_midnight_scheduler(
            self.tray_usage_service.clone(),
            publish,
        ));
        true
    }

    pub async fn stop_midnight_scheduler(&self) {
        if let Some(scheduler) = self.take_midnight_scheduler() {
            scheduler.stop().await;
        }
    }

    pub(crate) fn take_midnight_scheduler(&self) -> Option<TrayUsageSchedulerHandle> {
        match self.midnight_scheduler.lock() {
            Ok(mut scheduler) => scheduler.take(),
            Err(_) => {
                log::error!("tray usage midnight scheduler lock is poisoned");
                None
            }
        }
    }

    pub fn start_official_pricing_scheduler(&self) -> bool {
        let Ok(mut scheduler) = self.official_pricing_scheduler.lock() else {
            log::error!("official pricing scheduler lock is poisoned");
            return false;
        };
        if scheduler.is_some() {
            return false;
        }
        *scheduler = Some(start_official_pricing_scheduler(self.db.clone()));
        true
    }

    pub(crate) fn take_official_pricing_scheduler(&self) -> Option<OfficialPricingSchedulerHandle> {
        match self.official_pricing_scheduler.lock() {
            Ok(mut scheduler) => scheduler.take(),
            Err(_) => {
                log::error!("official pricing scheduler lock is poisoned");
                None
            }
        }
    }

    pub fn start_provider_key_usage_scheduler(&self) -> bool {
        let Ok(mut scheduler) = self.provider_key_usage_scheduler.lock() else {
            log::error!("provider key usage scheduler lock is poisoned");
            return false;
        };
        if scheduler.is_some() {
            return false;
        }
        *scheduler = Some(start_provider_key_usage_scheduler(
            self.binding_credential_service.clone(),
            self.system_provider_connection_service.clone(),
        ));
        true
    }

    pub(crate) fn take_provider_key_usage_scheduler(
        &self,
    ) -> Option<ProviderKeyUsageSchedulerHandle> {
        match self.provider_key_usage_scheduler.lock() {
            Ok(mut scheduler) => scheduler.take(),
            Err(_) => {
                log::error!("provider key usage scheduler lock is poisoned");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::subscription::SubscriptionQuota;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput, UsageProviderStored};
    use crate::usage::quota::QuotaCollector;
    use chrono::{Local, TimeZone};
    use futures::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingCollector {
        calls: AtomicUsize,
    }

    impl QuotaCollector for CountingCollector {
        fn source(&self) -> &'static str {
            "store_fake"
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a UsageProviderStored,
            _interactive: bool,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("store fake collection failure".to_string()) })
        }
    }

    #[tokio::test]
    async fn tray_usage_reuses_the_exact_injected_quota_and_session_services() {
        let db = Arc::new(Database::memory().unwrap());
        for provider in db.list_usage_providers().unwrap() {
            db.set_usage_provider_enabled(&provider.id, false).unwrap();
        }
        db.save_usage_provider(&UsageProviderInput {
            id: "store-injected-subscription".to_string(),
            name: "Store injected subscription".to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "store-injected".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: Some("store_fake".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();
        let collector = Arc::new(CountingCollector {
            calls: AtomicUsize::new(0),
        });
        let quota_service = Arc::new(QuotaService::with_collectors(
            db.clone(),
            vec![collector.clone()],
        ));
        let state = AppState::new_with_credential_store_and_quota_service(
            db,
            unavailable_credential_store(),
            quota_service,
        );

        assert!(state
            .tray_usage_service
            .shares_production_services(&state.quota_service, &state.session_usage_service,));
        state
            .tray_usage_service
            .refresh_at(
                Local.timestamp_opt(1_900_000_000, 0).single().unwrap(),
                |_| {},
            )
            .await;
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_handles_start_once_stop_idempotently_and_can_restart() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));

        assert!(state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
        assert!(!state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
        state.stop_quota_scheduler().await;
        state.stop_quota_scheduler().await;
        assert!(state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
        state.stop_quota_scheduler().await;

        let publisher: TraySnapshotPublisher = Arc::new(|_| {});
        assert!(state.start_midnight_scheduler(publisher.clone()));
        assert!(!state.start_midnight_scheduler(publisher.clone()));
        state.stop_midnight_scheduler().await;
        state.stop_midnight_scheduler().await;
        assert!(state.start_midnight_scheduler(publisher));
        state.stop_midnight_scheduler().await;
    }

    #[tokio::test]
    async fn scheduler_handles_can_be_detached_before_awaiting_shutdown() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let publisher: TraySnapshotPublisher = Arc::new(|_| {});
        assert!(state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
        assert!(state.start_midnight_scheduler(publisher.clone()));

        let quota = state
            .take_quota_scheduler()
            .expect("quota scheduler should detach");
        let midnight = state
            .take_midnight_scheduler()
            .expect("midnight scheduler should detach");

        assert!(state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
        assert!(state.start_midnight_scheduler(publisher));
        quota.stop().await;
        midnight.stop().await;
        state.stop_quota_scheduler().await;
        state.stop_midnight_scheduler().await;
    }
}
