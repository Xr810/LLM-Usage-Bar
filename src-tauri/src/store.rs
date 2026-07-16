use crate::credentials::{unavailable_credential_store, BindingCredentialService, CredentialStore};
use crate::database::Database;
#[cfg(test)]
use crate::services::claude_cli_auth::ClaudeAuthCommandRunner;
use crate::services::{
    ClaudeCliAuthService, ProxyService, SystemProviderConnectionService, UsageCache,
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
