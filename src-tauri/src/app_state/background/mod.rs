//! Application-owned scheduler handles and ordered shutdown.
mod runtime;
mod slot;
#[cfg(test)]
mod tests;
use crate::providers::shared::{
    key_usage_scheduler::ProviderKeyUsageSchedulerHandle,
    official_pricing::OfficialPricingSchedulerHandle,
};
use crate::quota::QuotaSchedulerHandle;
use crate::usage::tray_usage_scheduler::TrayUsageSchedulerHandle;
use slot::TaskSlot;

pub(super) struct BackgroundTasks {
    runtime: runtime::RuntimeTasks<tauri::async_runtime::JoinHandle<()>>,
    pub(super) quota: TaskSlot<QuotaSchedulerHandle>,
    pub(super) midnight: TaskSlot<TrayUsageSchedulerHandle>,
    pub(super) official_pricing: TaskSlot<OfficialPricingSchedulerHandle>,
    pub(super) provider_key_usage: TaskSlot<ProviderKeyUsageSchedulerHandle>,
}

impl Default for BackgroundTasks {
    fn default() -> Self {
        Self {
            runtime: runtime::RuntimeTasks::default(),
            quota: TaskSlot::new("quota scheduler"),
            midnight: TaskSlot::new("tray usage midnight scheduler"),
            official_pricing: TaskSlot::new("official pricing scheduler"),
            provider_key_usage: TaskSlot::new("provider key usage scheduler"),
        }
    }
}

impl BackgroundTasks {
    pub(super) fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(|| tauri::async_runtime::spawn(task));
    }

    pub(super) fn take(&self) -> BackgroundShutdown {
        let runtime = self.runtime.close();
        for task in &runtime {
            task.abort();
        }
        BackgroundShutdown {
            runtime,
            quota: self.quota.close(),
            midnight: self.midnight.close(),
            official_pricing: self.official_pricing.close(),
            provider_key_usage: self.provider_key_usage.close(),
        }
    }
}

/// Owned handles: no AppState borrow or mutex guard survives into shutdown.
pub(crate) struct BackgroundShutdown {
    runtime: Vec<tauri::async_runtime::JoinHandle<()>>,
    quota: Option<QuotaSchedulerHandle>,
    midnight: Option<TrayUsageSchedulerHandle>,
    official_pricing: Option<OfficialPricingSchedulerHandle>,
    provider_key_usage: Option<ProviderKeyUsageSchedulerHandle>,
}

impl BackgroundShutdown {
    pub(crate) async fn stop(self) {
        for task in self.runtime {
            // Cancellation is expected at application shutdown. Awaiting also
            // lets any currently executing synchronous database step finish.
            let _ = task.await;
        }
        // Startup can have registered a watcher just before cancellation.
        crate::usage::watcher::stop_usage_watcher();
        if let Some(scheduler) = self.quota {
            scheduler.stop().await;
        }
        if let Some(scheduler) = self.midnight {
            scheduler.stop().await;
        }
        if let Some(scheduler) = self.official_pricing {
            scheduler.stop().await;
        }
        if let Some(scheduler) = self.provider_key_usage {
            scheduler.stop().await;
        }
    }
}
