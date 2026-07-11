use crate::database::Database;
use crate::services::{ProxyService, UsageCache};
use crate::usage::quota::{QuotaSchedulerHandle, QuotaService};
use crate::usage::session::SessionUsageService;
use std::sync::{Arc, Mutex};

/// 全局应用状态
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
    pub usage_cache: Arc<UsageCache>,
    pub quota_service: Arc<QuotaService>,
    pub session_usage_service: Arc<SessionUsageService>,
    quota_scheduler: Mutex<Option<QuotaSchedulerHandle>>,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        let proxy_service = ProxyService::new(db.clone());
        let quota_service = Arc::new(QuotaService::new(db.clone()));
        let session_usage_service = Arc::new(SessionUsageService::new(db.clone()));

        Self {
            db,
            proxy_service,
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
