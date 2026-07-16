use crate::credentials::{unavailable_credential_store, BindingCredentialService, CredentialStore};
use crate::database::Database;
#[cfg(test)]
use crate::services::claude_cli_auth::ClaudeAuthCommandRunner;
use crate::services::{
    tray_usage::TrayUsageService, ClaudeCliAuthService, ProxyService,
    SystemProviderConnectionService, UsageCache,
};
use crate::usage::quota::{QuotaSchedulerHandle, QuotaService};
use crate::usage::session::SessionUsageService;
use std::sync::{Arc, Mutex};

/// 全局应用状态
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
    pub credential_store: Arc<dyn CredentialStore>,
    pub binding_credential_service: Arc<BindingCredentialService>,
    pub claude_cli_auth_service: Arc<ClaudeCliAuthService>,
    pub system_provider_connection_service: Arc<SystemProviderConnectionService>,
    pub usage_cache: Arc<UsageCache>,
    pub quota_service: Arc<QuotaService>,
    pub session_usage_service: Arc<SessionUsageService>,
    pub tray_usage_service: Arc<TrayUsageService>,
    quota_scheduler: Mutex<Option<QuotaSchedulerHandle>>,
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
        let proxy_service =
            ProxyService::new_with_credential_store(db.clone(), credential_store.clone());
        let binding_credential_service = proxy_service.binding_credential_service();
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
            proxy_service,
            credential_store,
            binding_credential_service,
            claude_cli_auth_service,
            system_provider_connection_service,
            usage_cache: Arc::new(UsageCache::new()),
            quota_service,
            session_usage_service,
            tray_usage_service,
            quota_scheduler: Mutex::new(None),
        }
    }

    pub fn start_quota_scheduler(&self) {
        let Ok(mut scheduler) = self.quota_scheduler.lock() else {
            log::error!("quota scheduler lock is poisoned");
            return;
        };
        if scheduler.is_none() {
            *scheduler = Some(self.quota_service.clone().start_scheduler());
        }
    }

    pub async fn stop_quota_scheduler(&self) {
        let scheduler = match self.quota_scheduler.lock() {
            Ok(mut scheduler) => scheduler.take(),
            Err(_) => {
                log::error!("quota scheduler lock is poisoned");
                None
            }
        };
        if let Some(scheduler) = scheduler {
            scheduler.stop().await;
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
}
