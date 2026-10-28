#[cfg(test)]
use crate::providers::claude::cli_auth::ClaudeAuthCommandRunner;
mod background;
use crate::providers::{
    shared::key_usage_scheduler::start_scheduler as start_provider_key_usage_scheduler,
    shared::official_pricing::start_scheduler as start_official_pricing_scheduler,
    ClaudeCliAuthService, SystemProviderConnectionService,
};
use crate::quota::{QuotaCycleCallback, QuotaSchedulerHandle, QuotaService};
use crate::route::takeover::ToolDetectionCache;
use crate::secrets::{unavailable_credential_store, BindingCredentialService, CredentialStore};
use crate::store::Database;
use crate::usage::session::SessionUsageService;
use crate::usage::tray_usage::TrayUsageService;
use crate::usage::tray_usage_scheduler::{
    start_local_midnight_scheduler, TraySnapshotPublisher, TrayUsageSchedulerHandle,
};
use crate::usage::usage_cache::UsageCache;
pub(crate) use background::BackgroundShutdown;
use background::BackgroundTasks;
use std::sync::Arc;

type DatabaseFactory<T> = Box<dyn FnOnce(Arc<Database>) -> Result<Arc<T>, crate::AppError> + Send>;

/// Build-owned factories, called once with the migrated authoritative database.
/// License runs first. Either failure discards both providers, closes admission,
/// and disables configured cleanup for this run. Factory errors are not echoed.
pub struct Injections {
    pub license: DatabaseFactory<dyn crate::policy::LicenseProvider>,
    pub pending_registry: DatabaseFactory<dyn crate::policy::PendingRegistry>,
    pub gate: Arc<dyn crate::policy::PolicyGate>,
    pub retention: DatabaseFactory<dyn crate::store::RetentionPolicyProvider>,
}

impl Injections {
    pub fn community() -> Self {
        Self {
            license: Box::new(|_| Ok(Arc::new(crate::policy::CommunityLicenseProvider))),
            pending_registry: Box::new(|_| Ok(Arc::new(crate::policy::DefaultNoopRegistry))),
            gate: Arc::new(crate::policy::AllowAllGate),
            retention: Box::new(|_| Ok(Arc::new(crate::store::CommunityRetentionPolicy))),
        }
    }
}

struct UnavailableGate;

impl crate::policy::PolicyGate for UnavailableGate {
    fn admit(&self, _: &crate::policy::ReqMeta) -> crate::policy::Decision {
        crate::policy::Decision::Hold {
            reason: "Build dependencies unavailable".into(),
            retry_after_seconds: None,
            hold: None,
        }
    }

    fn settle(&self, _: &str, _: &crate::policy::Usage) {}
}

/// 全局应用状态
pub struct AppState {
    pub db: Arc<Database>,
    pub license: Arc<dyn crate::policy::LicenseProvider>,
    pub license_lifecycle: Arc<crate::policy::LicenseLifecycle>,
    pub gate: Arc<dyn crate::policy::PolicyGate>,
    pub pending_registry: Arc<dyn crate::policy::PendingRegistry>,
    // Keep retention alive without a Database -> provider -> Database cycle.
    _retention: Option<Arc<dyn crate::store::RetentionPolicyProvider>>,
    pub credential_store: Arc<dyn CredentialStore>,
    pub binding_credential_service: Arc<BindingCredentialService>,
    pub claude_cli_auth_service: Arc<ClaudeCliAuthService>,
    pub system_provider_connection_service: Arc<SystemProviderConnectionService>,
    pub usage_cache: Arc<UsageCache>,
    pub quota_service: Arc<QuotaService>,
    pub session_usage_service: Arc<SessionUsageService>,
    pub tray_usage_service: Arc<TrayUsageService>,
    pub tool_detection_cache: Arc<ToolDetectionCache>,
    background: BackgroundTasks,
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
            Injections::community(),
        )
    }

    pub fn new_with_credential_store_and_quota_service(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        quota_service: Arc<QuotaService>,
    ) -> Self {
        Self::new_with_injections(db, credential_store, quota_service, Injections::community())
    }

    pub fn new_with_injections(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        quota_service: Arc<QuotaService>,
        injections: Injections,
    ) -> Self {
        Self::new_with_services(
            db,
            credential_store,
            Arc::new(ClaudeCliAuthService::production()),
            quota_service,
            injections,
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
            Injections::community(),
        )
    }

    fn new_with_services(
        db: Arc<Database>,
        credential_store: Arc<dyn CredentialStore>,
        claude_cli_auth_service: Arc<ClaudeCliAuthService>,
        quota_service: Arc<QuotaService>,
        injections: Injections,
    ) -> Self {
        let pending_registry = (injections.pending_registry)(db.clone()).unwrap_or_else(|_| {
            log::error!("Pending registry initialization failed; using community registry.");
            Arc::new(crate::policy::DefaultNoopRegistry)
        });
        let resolved = (injections.license)(db.clone()).and_then(|license| {
            let retention = (injections.retention)(db.clone())?;
            // An injected Pro entitlement must survive a downgrade before the
            // first prune. Persist its floor before exposing either provider.
            if license.status().tier == crate::policy::LicenseTier::Pro {
                crate::policy::RetentionStore::new(db.clone()).set_pending_target(730)?;
            }
            Ok((license, retention))
        });
        let (license, gate, retention) = match resolved {
            Ok((license, retention)) => (license, injections.gate, Some(retention)),
            Err(_) => {
                // Factory errors can contain build-private material; never echo them.
                log::error!("Build dependency initialization failed; community license, closed gate, cleanup disabled.");
                (
                    Arc::new(crate::policy::CommunityLicenseProvider)
                        as Arc<dyn crate::policy::LicenseProvider>,
                    Arc::new(UnavailableGate) as Arc<dyn crate::policy::PolicyGate>,
                    None,
                )
            }
        };
        db.install_retention_policy(retention.as_ref());
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
        let tool_detection_cache = Arc::new(ToolDetectionCache::default());
        let license_lifecycle = Arc::new(crate::policy::LicenseLifecycle::new(license, db.clone()));
        let tray_usage_service = Arc::new(TrayUsageService::new(
            db.clone(),
            quota_service.clone(),
            session_usage_service.clone(),
            tool_detection_cache.clone(),
            license_lifecycle.clone(),
        ));

        Self {
            db,
            license: license_lifecycle.clone(),
            license_lifecycle,
            gate,
            pending_registry,
            _retention: retention,
            credential_store,
            binding_credential_service,
            claude_cli_auth_service,
            system_provider_connection_service,
            usage_cache: Arc::new(UsageCache::new()),
            quota_service,
            session_usage_service,
            tray_usage_service,
            tool_detection_cache,
            background: BackgroundTasks::default(),
        }
    }

    pub fn start_quota_scheduler(&self, after_cycle: QuotaCycleCallback) -> bool {
        self.background
            .quota
            .start(|| self.quota_service.clone().start_scheduler(after_cycle))
    }

    pub async fn stop_quota_scheduler(&self) {
        if let Some(scheduler) = self.take_quota_scheduler() {
            scheduler.stop().await;
        }
    }

    pub(crate) fn take_quota_scheduler(&self) -> Option<QuotaSchedulerHandle> {
        self.background.quota.take()
    }

    pub fn start_midnight_scheduler(&self, publish: TraySnapshotPublisher) -> bool {
        self.background
            .midnight
            .start(|| start_local_midnight_scheduler(self.tray_usage_service.clone(), publish))
    }

    pub async fn stop_midnight_scheduler(&self) {
        if let Some(scheduler) = self.take_midnight_scheduler() {
            scheduler.stop().await;
        }
    }

    pub(crate) fn take_midnight_scheduler(&self) -> Option<TrayUsageSchedulerHandle> {
        self.background.midnight.take()
    }

    pub fn start_official_pricing_scheduler(&self) -> bool {
        self.background
            .official_pricing
            .start(|| start_official_pricing_scheduler(self.db.clone()))
    }

    pub fn start_provider_key_usage_scheduler(&self) -> bool {
        self.background.provider_key_usage.start(|| {
            start_provider_key_usage_scheduler(
                self.binding_credential_service.clone(),
                self.system_provider_connection_service.clone(),
            )
        })
    }

    pub(crate) fn spawn_background(
        &self,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        self.background.spawn(task);
    }

    pub(crate) fn take_background_tasks(&self) -> BackgroundShutdown {
        self.background.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BillingKind, TokenSource, UsageProviderInput, UsageProviderStored};
    use crate::providers::SubscriptionQuota;
    use crate::quota::QuotaCollector;
    use chrono::{Local, TimeZone};
    use futures::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn deferred_factories_share_migrated_database_and_fail_closed_once() {
        // Isolate the process-global logger from other test modules.
        const CHILD: &str = "LUB_DEFERRED_FACTORY_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "app_state::tests::deferred_factories_share_migrated_database_and_fail_closed_once", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        struct Logger(std::sync::Mutex<Vec<String>>);
        impl log::Log for Logger {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                if record.level() == log::Level::Error {
                    self.0.lock().unwrap().push(record.args().to_string());
                }
            }
            fn flush(&self) {}
        }
        static LOGGER: Logger = Logger(std::sync::Mutex::new(Vec::new()));
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Error);

        // Test-only official provider owning the DB exercises cycle-free shutdown.
        struct TestProvider(Arc<Database>);
        impl crate::policy::LicenseProvider for TestProvider {
            fn status(&self) -> crate::policy::LicenseStatus {
                let mut status = crate::policy::community_license_status();
                status.build = crate::policy::LicenseBuild::Official;
                status.tier = crate::policy::LicenseTier::Pro;
                status
            }
            fn activate(
                &self,
                token: &str,
            ) -> Result<crate::policy::LicenseStatus, crate::AppError> {
                crate::policy::activate_community_license(token)
            }
            fn deactivate(&self) -> Result<(), crate::AppError> {
                crate::policy::deactivate_community_license()
            }
        }
        impl crate::RetentionPolicyProvider for TestProvider {
            fn retention_policy(&self, detail_days: i64) -> crate::RetentionPolicy {
                assert!(self.0.database_path().is_some());
                crate::RetentionPolicy {
                    detail_days,
                    hourly_days: 730,
                }
            }
        }
        for mode in ["community", "success", "license-error", "retention-error"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("authoritative.db");
            let db = Arc::new(Database::open_before_retention(&path).unwrap());
            for days in [500, 800] {
                db.conn.lock().unwrap().execute(
                    "INSERT INTO usage_hourly_rollups
                     (hour_utc, app_type, provider_id, model, billing_identity, data_source, request_count)
                     VALUES (?1, 'claude', 'factory-test', 'model', '', 'proxy', ?2)",
                    rusqlite::params![(chrono::Utc::now() - chrono::Duration::days(days)).timestamp().div_euclid(3600) * 3600, days],
                ).unwrap();
            }
            // Reopen through the exact production opener: it must not prune yet.
            drop(db);
            let db = Arc::new(Database::open_before_retention(&path).unwrap());
            let weak = Arc::downgrade(&db);
            let license_db = weak.clone();
            let retention_db = weak.clone();
            let mut injections = Injections::community();
            if mode != "community" {
                injections.license = Box::new(move |db| {
                    assert!(Arc::ptr_eq(&db, &license_db.upgrade().unwrap()));
                    let conn = db.conn.lock().unwrap();
                    assert_eq!(
                        Database::get_user_version(&conn).unwrap(),
                        crate::store::SCHEMA_VERSION
                    );
                    assert_eq!(
                        conn.query_row("SELECT COUNT(*) FROM usage_hourly_rollups", [], |row| row
                            .get::<_, i64>(
                            0
                        ))
                        .unwrap(),
                        2
                    );
                    drop(conn);
                    if mode == "license-error" {
                        Err(crate::AppError::internal("private factory failure detail"))
                    } else {
                        Ok(Arc::new(TestProvider(db)))
                    }
                });
                injections.retention = Box::new(move |db| {
                    assert_ne!(
                        mode, "license-error",
                        "retention must not run after license failure"
                    );
                    assert!(Arc::ptr_eq(&db, &retention_db.upgrade().unwrap()));
                    if mode == "retention-error" {
                        Err(crate::AppError::internal("private factory failure detail"))
                    } else {
                        Ok(Arc::new(TestProvider(db)))
                    }
                });
            }
            LOGGER.0.lock().unwrap().clear();
            let state = AppState::new_with_injections(
                db.clone(),
                unavailable_credential_store(),
                Arc::new(QuotaService::new(db.clone())),
                injections,
            );
            state.db.startup_cleanup();
            assert!(Arc::ptr_eq(&state.db, &db));
            if mode == "success" {
                assert_eq!(
                    state.license.status().build,
                    crate::policy::LicenseBuild::Official
                );
                assert_eq!(state.license.status().tier, crate::policy::LicenseTier::Pro);
            } else {
                assert_eq!(
                    state.license.status(),
                    crate::policy::community_license_status()
                );
            }
            state.license_lifecycle.reconcile().unwrap();
            let failed = mode.ends_with("error");
            let request = crate::policy::ReqMeta {
                req_id: "factory-test".into(),
                app_type: "claude".into(),
                provider_id: "p".into(),
                model: "m".into(),
                billing_identity: crate::policy::BillingIdentity::ApiKey,
                key_fp: None,
                session_id: None,
                est_cost_micro_usd: 1,
                via_subscription_optin: false,
            };
            if failed {
                assert!(matches!(
                    state.gate.admit(&request),
                    crate::policy::Decision::Hold { .. }
                ));
                assert!(db.rollup_and_prune_configured().is_err());
            } else {
                assert_eq!(state.gate.admit(&request), crate::policy::Decision::Allow);
                db.rollup_and_prune_configured().unwrap();
            }
            let total: i64 = db
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COALESCE(SUM(request_count), 0) FROM usage_hourly_rollups",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                total,
                match mode {
                    "success" => 500,
                    "community" => 0,
                    _ => 1300,
                },
                "{mode}"
            );
            let logs = LOGGER.0.lock().unwrap();
            assert_eq!(logs.len(), usize::from(failed), "{mode}: {logs:?}");
            assert!(logs
                .iter()
                .all(|line| !line.contains("private factory failure detail")));
            drop(logs);
            drop(state);
            // Scheduled cleanup must not silently revert to community after teardown.
            assert!(db.rollup_and_prune_configured().is_err());
            drop(db);
            assert!(
                weak.upgrade().is_none(),
                "provider must not leak the database"
            );
        }
    }

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
