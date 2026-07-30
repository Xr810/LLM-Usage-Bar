use crate::database::Database;
use crate::error::AppError;
use crate::usage::domain::{BillingKind, UsageProviderView};
use crate::usage::quota::QuotaService;
use crate::usage::session::{ProviderSessionSyncResult, SessionUsageService};
use crate::usage::status::UsageStatus;
use crate::usage::tray_snapshot::{TrayUsageProjector, TrayUsageSnapshot};
use chrono::{DateTime, Local};
use futures::future::join_all;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub(crate) type ProjectSnapshotFn =
    Arc<dyn Fn(DateTime<Local>) -> Result<TrayUsageSnapshot, AppError> + Send + Sync>;
pub(crate) type RefreshSourcesFuture = Pin<Box<dyn Future<Output = Result<(), ()>> + Send>>;
pub(crate) type RefreshSourcesFn = Arc<dyn Fn() -> RefreshSourcesFuture + Send + Sync>;
pub(crate) type NowFn = Arc<dyn Fn() -> DateTime<Local> + Send + Sync>;

type ProviderAttemptFuture<T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send>>;
type QuotaAttemptFn = Arc<dyn Fn(String) -> ProviderAttemptFuture<()> + Send + Sync>;
type SessionAttemptFn =
    Arc<dyn Fn(String) -> ProviderAttemptFuture<ProviderSessionSyncResult> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefreshCandidate {
    provider_id: String,
    quota: bool,
    session: bool,
}

fn select_refresh_candidates(providers: &[UsageProviderView]) -> Vec<RefreshCandidate> {
    providers
        .iter()
        .filter_map(|provider| {
            if !provider.enabled {
                return None;
            }
            let quota = provider.billing_kind == BillingKind::Subscription
                && provider.quota_source.is_some();
            let session = !provider.session_source_bindings.is_empty();
            (quota || session).then(|| RefreshCandidate {
                provider_id: provider.id.clone(),
                quota,
                session,
            })
        })
        .collect()
}

async fn execute_refresh_candidates(
    candidates: Vec<RefreshCandidate>,
    quota_attempt: QuotaAttemptFn,
    session_attempt: SessionAttemptFn,
) -> Result<(), ()> {
    let mut attempts = Vec::<Pin<Box<dyn Future<Output = bool> + Send>>>::new();
    for candidate in candidates {
        if candidate.quota {
            let future = quota_attempt(candidate.provider_id.clone());
            attempts.push(Box::pin(async move { future.await.is_ok() }));
        }
        if candidate.session {
            let future = session_attempt(candidate.provider_id);
            attempts.push(Box::pin(async move {
                future.await.is_ok_and(|result| result.errors.is_empty())
            }));
        }
    }

    if join_all(attempts).await.into_iter().all(|success| success) {
        Ok(())
    } else {
        Err(())
    }
}

async fn refresh_all_sources(
    db: Arc<Database>,
    quota_service: Arc<QuotaService>,
    session_usage_service: Arc<SessionUsageService>,
) -> Result<(), ()> {
    let providers = db.list_usage_providers().map_err(|_| ())?;
    let candidates = select_refresh_candidates(&providers);
    let quota_attempt: QuotaAttemptFn = Arc::new(move |provider_id| {
        let quota_service = quota_service.clone();
        Box::pin(async move {
            quota_service
                .refresh_provider(&provider_id)
                .await
                .map(|_| ())
        })
    });
    let session_attempt: SessionAttemptFn = Arc::new(move |provider_id| {
        let session_usage_service = session_usage_service.clone();
        Box::pin(async move {
            match tauri::async_runtime::spawn_blocking(move || {
                session_usage_service.sync_provider(&provider_id)
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(AppError::Message("session refresh task failed".to_string())),
            }
        })
    });

    execute_refresh_candidates(candidates, quota_attempt, session_attempt).await
}

struct RefreshLease {
    active: Arc<AtomicBool>,
    armed: bool,
}

impl RefreshLease {
    fn finish(&mut self) {
        if self.armed {
            let _ = self
                .active
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire);
            self.armed = false;
        }
    }
}

impl Drop for RefreshLease {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .active
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

enum CommitMode<'a> {
    Ordinary,
    RefreshTerminal(&'a mut RefreshLease),
}

enum FinalTime {
    Clock,
    Fixed(DateTime<Local>),
}

pub struct TrayUsageService {
    project_snapshot: ProjectSnapshotFn,
    refresh_sources: RefreshSourcesFn,
    now: NowFn,
    cache: RwLock<TrayUsageSnapshot>,
    commit_order: Mutex<()>,
    refresh_active: Arc<AtomicBool>,
    #[cfg(test)]
    production_quota_service: Option<Arc<QuotaService>>,
    #[cfg(test)]
    production_session_usage_service: Option<Arc<SessionUsageService>>,
}

impl TrayUsageService {
    pub fn new(
        db: Arc<Database>,
        quota_service: Arc<QuotaService>,
        session_usage_service: Arc<SessionUsageService>,
    ) -> Self {
        #[cfg(test)]
        let production_quota_service = quota_service.clone();
        #[cfg(test)]
        let production_session_usage_service = session_usage_service.clone();
        let projector = Arc::new(TrayUsageProjector::new(db.clone()));
        let project_snapshot: ProjectSnapshotFn = Arc::new(move |now| projector.project_at(now));
        let refresh_sources: RefreshSourcesFn = Arc::new(move || {
            let db = db.clone();
            let quota_service = quota_service.clone();
            let session_usage_service = session_usage_service.clone();
            Box::pin(
                async move { refresh_all_sources(db, quota_service, session_usage_service).await },
            )
        });
        let service =
            Self::new_with_dependencies(project_snapshot, refresh_sources, Arc::new(Local::now));
        #[cfg(test)]
        let service = {
            let mut service = service;
            service.production_quota_service = Some(production_quota_service);
            service.production_session_usage_service = Some(production_session_usage_service);
            service
        };
        service
    }

    pub(crate) fn new_with_dependencies(
        project_snapshot: ProjectSnapshotFn,
        refresh_sources: RefreshSourcesFn,
        now: NowFn,
    ) -> Self {
        Self {
            project_snapshot,
            refresh_sources,
            now,
            cache: RwLock::new(TrayUsageSnapshot::unknown(0)),
            commit_order: Mutex::new(()),
            refresh_active: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            production_quota_service: None,
            #[cfg(test)]
            production_session_usage_service: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn shares_production_services(
        &self,
        quota_service: &Arc<QuotaService>,
        session_usage_service: &Arc<SessionUsageService>,
    ) -> bool {
        self.production_quota_service
            .as_ref()
            .is_some_and(|service| Arc::ptr_eq(service, quota_service))
            && self
                .production_session_usage_service
                .as_ref()
                .is_some_and(|service| Arc::ptr_eq(service, session_usage_service))
    }

    pub async fn cached_snapshot(&self) -> TrayUsageSnapshot {
        let mut snapshot = self.cache.read().await.clone();
        snapshot.refresh_in_progress = self.refresh_active.load(Ordering::Acquire);
        snapshot
    }

    pub async fn refresh<F>(&self, publish: F) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        self.refresh_with_time(FinalTime::Clock, publish).await
    }

    pub async fn refresh_at<F>(&self, now: DateTime<Local>, publish: F) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        self.refresh_with_time(FinalTime::Fixed(now), publish).await
    }

    pub async fn rebuild_from_persisted<F>(&self, publish: F) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let _order = self.commit_order.lock().await;
        let now = (self.now)();
        self.project_and_commit_locked(now, CommitMode::Ordinary, &publish)
            .await
    }

    pub async fn rebuild_from_persisted_at<F>(
        &self,
        now: DateTime<Local>,
        publish: F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let _order = self.commit_order.lock().await;
        self.project_and_commit_locked(now, CommitMode::Ordinary, &publish)
            .await
    }

    pub async fn mark_refresh_failed_at<F>(
        &self,
        generated_at: i64,
        publish: F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let _order = self.commit_order.lock().await;
        self.commit_failure_locked(generated_at, CommitMode::Ordinary, &publish)
            .await
    }

    async fn refresh_with_time<F>(&self, final_time: FinalTime, publish: F) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let Some(mut lease) = self.try_begin_refresh() else {
            return self.cached_snapshot().await;
        };

        self.publish_refresh_started(&publish).await;
        let sources_succeeded = (self.refresh_sources)().await.is_ok();
        let _order = self.commit_order.lock().await;
        let now = match final_time {
            FinalTime::Clock => (self.now)(),
            FinalTime::Fixed(now) => now,
        };
        if sources_succeeded {
            self.project_and_commit_locked(now, CommitMode::RefreshTerminal(&mut lease), &publish)
                .await
        } else {
            self.commit_failure_locked(
                now.timestamp(),
                CommitMode::RefreshTerminal(&mut lease),
                &publish,
            )
            .await
        }
    }

    fn try_begin_refresh(&self) -> Option<RefreshLease> {
        self.refresh_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RefreshLease {
                active: self.refresh_active.clone(),
                armed: true,
            })
    }

    async fn publish_refresh_started<F>(&self, publish: &F)
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let _order = self.commit_order.lock().await;
        let published = {
            let mut cache = self.cache.write().await;
            cache.refresh_in_progress = self.refresh_active.load(Ordering::Acquire);
            cache.refresh_error = None;
            cache.clone()
        };
        publish(&published);
    }

    async fn project_and_commit_locked<F>(
        &self,
        now: DateTime<Local>,
        mode: CommitMode<'_>,
        publish: &F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let generated_at = now.timestamp();
        match (self.project_snapshot)(now) {
            Ok(candidate) => self.commit_success_locked(candidate, mode, publish).await,
            Err(_) => {
                self.commit_failure_locked(generated_at, mode, publish)
                    .await
            }
        }
    }

    async fn commit_success_locked<F>(
        &self,
        mut candidate: TrayUsageSnapshot,
        mode: CommitMode<'_>,
        publish: &F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let terminal = matches!(&mode, CommitMode::RefreshTerminal(_));
        let published = {
            let mut cache = self.cache.write().await;
            candidate.last_success_at = if candidate.status == UsageStatus::Unknown {
                cache.last_success_at
            } else {
                Some(candidate.generated_at)
            };
            candidate.stale = false;
            candidate.refresh_error = None;
            candidate.refresh_in_progress = if terminal {
                false
            } else {
                self.refresh_active.load(Ordering::Acquire)
            };
            *cache = candidate;
            cache.clone()
        };
        Self::finish_terminal(mode);
        publish(&published);
        published
    }

    async fn commit_failure_locked<F>(
        &self,
        generated_at: i64,
        mode: CommitMode<'_>,
        publish: &F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot) + Send + Sync,
    {
        let terminal = matches!(&mode, CommitMode::RefreshTerminal(_));
        let published = {
            let mut cache = self.cache.write().await;
            cache.generated_at = generated_at;
            cache.stale = true;
            cache.refresh_error = Some("tray_usage_refresh_failed".to_string());
            cache.refresh_in_progress = if terminal {
                false
            } else {
                self.refresh_active.load(Ordering::Acquire)
            };
            cache.clone()
        };
        Self::finish_terminal(mode);
        publish(&published);
        published
    }

    fn finish_terminal(mode: CommitMode<'_>) {
        if let CommitMode::RefreshTerminal(lease) = mode {
            lease.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::domain::{BindingCredentialStatus, TokenSource};
    use crate::usage::tray_snapshot::TrayAgentUsageView;
    use chrono::TimeZone;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    const TEST_TIMEOUT: Duration = Duration::from_secs(2);

    fn local_time(timestamp: i64) -> DateTime<Local> {
        Local.timestamp_opt(timestamp, 0).single().unwrap()
    }

    fn fixed_now(timestamp: i64) -> NowFn {
        Arc::new(move || local_time(timestamp))
    }

    fn snapshot(status: UsageStatus, generated_at: i64, row: &str) -> TrayUsageSnapshot {
        TrayUsageSnapshot {
            status,
            generated_at,
            last_success_at: Some(generated_at),
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            api_budget: crate::usage::tray_snapshot::TrayApiBudgetView::default(),
            agents: vec![TrayAgentUsageView {
                agent_module_id: row.to_string(),
                name: row.to_string(),
                sort_order: 0,
                status,
                providers: Vec::new(),
            }],
        }
    }

    fn queue_projector(projections: Vec<Result<TrayUsageSnapshot, AppError>>) -> ProjectSnapshotFn {
        let projections = Arc::new(StdMutex::new(VecDeque::from(projections)));
        Arc::new(move |_| {
            projections
                .lock()
                .unwrap()
                .pop_front()
                .expect("queued projection")
        })
    }

    fn successful_sources() -> RefreshSourcesFn {
        Arc::new(|| Box::pin(async { Ok(()) }))
    }

    fn provider_view(
        id: &str,
        billing_kind: BillingKind,
        enabled: bool,
        quota_source: Option<&str>,
        session_source_bindings: &[&str],
    ) -> UsageProviderView {
        UsageProviderView {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind,
            product_group_id: id.to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: session_source_bindings
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            bindings: Vec::new(),
            quota_source: quota_source.map(str::to_string),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            enabled,
            needs_review: false,
            created_at: 0,
            updated_at: 0,
            route_base_url: None,
            has_route_credentials: false,
            system_preset_key: None,
            system_auth_kind: None,
            canonical_endpoint: None,
            compatible_agent_module_ids: Vec::new(),
            upstream_credential_status: BindingCredentialStatus::NotRequired,
            upstream_credential_version: 0,
            can_clear_upstream_credential: false,
            last_connection_test_at: None,
            last_connection_test_status: None,
            daily_budget_usd: None,
        }
    }

    #[tokio::test]
    async fn failed_refresh_keeps_the_entire_last_successful_snapshot_and_redacts_errors() {
        let service = TrayUsageService::new_with_dependencies(
            queue_projector(vec![
                Ok(snapshot(UsageStatus::Yellow, 1_000, "old-provider-row")),
                Err(AppError::Message(
                    "sk-secret raw upstream failure".to_string(),
                )),
            ]),
            successful_sources(),
            fixed_now(9_999),
        );

        let first = service
            .rebuild_from_persisted_at(local_time(1_000), |_| {})
            .await;
        let failed = service.refresh_at(local_time(2_000), |_| {}).await;

        assert_eq!(failed.status, UsageStatus::Yellow);
        assert_eq!(failed.agents, first.agents);
        assert_eq!(failed.last_success_at, first.last_success_at);
        assert_eq!(failed.generated_at, 2_000);
        assert!(failed.stale);
        assert_eq!(
            failed.refresh_error.as_deref(),
            Some("tray_usage_refresh_failed"),
        );
        assert!(!failed.refresh_in_progress);
        let serialized = serde_json::to_string(&failed).unwrap();
        assert!(!serialized.contains("sk-secret"), "{serialized}");
    }

    #[tokio::test]
    async fn source_and_projector_secrets_never_reach_snapshot_surfaces() {
        let source_calls = Arc::new(AtomicUsize::new(0));
        let refresh_sources: RefreshSourcesFn = {
            let source_calls = source_calls.clone();
            Arc::new(move || {
                let call = source_calls.fetch_add(1, AtomicOrdering::SeqCst);
                Box::pin(async move {
                    if call > 0 {
                        return Ok(());
                    }
                    let quota_attempt: QuotaAttemptFn = Arc::new(|_| {
                        Box::pin(async {
                            Err(AppError::Message("source-refresh-sk-secret".to_string()))
                        })
                    });
                    let session_attempt: SessionAttemptFn =
                        Arc::new(|_| Box::pin(async { Ok(ProviderSessionSyncResult::default()) }));
                    execute_refresh_candidates(
                        vec![RefreshCandidate {
                            provider_id: "secret-source".to_string(),
                            quota: true,
                            session: false,
                        }],
                        quota_attempt,
                        session_attempt,
                    )
                    .await
                })
            })
        };
        let service = TrayUsageService::new_with_dependencies(
            queue_projector(vec![
                Ok(snapshot(UsageStatus::Yellow, 1_000, "safe-row")),
                Err(AppError::Message("projector-refresh-sk-secret".to_string())),
            ]),
            refresh_sources,
            fixed_now(9_999),
        );
        service
            .rebuild_from_persisted_at(local_time(1_000), |_| {})
            .await;
        let published = Arc::new(StdMutex::new(Vec::new()));
        let source_failure = service
            .refresh_at(local_time(2_000), {
                let published = published.clone();
                move |snapshot| published.lock().unwrap().push(snapshot.clone())
            })
            .await;
        let projector_failure = service
            .refresh_at(local_time(3_000), {
                let published = published.clone();
                move |snapshot| published.lock().unwrap().push(snapshot.clone())
            })
            .await;
        let cached = service.cached_snapshot().await;
        let published = published.lock().unwrap().clone();

        let serialized =
            serde_json::to_string(&(source_failure, projector_failure, cached, published)).unwrap();
        assert!(
            !serialized.contains("source-refresh-sk-secret"),
            "{serialized}"
        );
        assert!(
            !serialized.contains("projector-refresh-sk-secret"),
            "{serialized}",
        );
    }

    #[tokio::test]
    async fn first_failure_unknown_recovery_and_unknown_last_success_rules_are_exact() {
        let source_calls = Arc::new(AtomicUsize::new(0));
        let refresh_sources: RefreshSourcesFn = {
            let source_calls = source_calls.clone();
            Arc::new(move || {
                let call = source_calls.fetch_add(1, AtomicOrdering::SeqCst);
                Box::pin(async move {
                    if call == 0 {
                        Err(())
                    } else {
                        Ok(())
                    }
                })
            })
        };
        let service = TrayUsageService::new_with_dependencies(
            queue_projector(vec![
                Ok(TrayUsageSnapshot::unknown(2_000)),
                Ok(snapshot(UsageStatus::Green, 3_000, "green")),
                Ok(TrayUsageSnapshot::unknown(4_000)),
            ]),
            refresh_sources,
            fixed_now(9_999),
        );

        let first_failure = service.refresh_at(local_time(1_000), |_| {}).await;
        assert_eq!(first_failure.status, UsageStatus::Unknown);
        assert_eq!(first_failure.last_success_at, None);
        assert!(first_failure.stale);

        let first_unknown = service
            .rebuild_from_persisted_at(local_time(2_000), |_| {})
            .await;
        assert_eq!(first_unknown.last_success_at, None);
        assert!(!first_unknown.stale);

        let recovered = service
            .rebuild_from_persisted_at(local_time(3_000), |_| {})
            .await;
        assert_eq!(recovered.last_success_at, Some(3_000));
        assert_eq!(recovered.refresh_error, None);

        let later_unknown = service
            .rebuild_from_persisted_at(local_time(4_000), |_| {})
            .await;
        assert_eq!(later_unknown.status, UsageStatus::Unknown);
        assert!(later_unknown.agents.is_empty());
        assert_eq!(later_unknown.last_success_at, Some(3_000));
    }

    #[tokio::test]
    async fn successful_projection_replaces_the_whole_cached_body() {
        let service = TrayUsageService::new_with_dependencies(
            queue_projector(vec![
                Ok(snapshot(UsageStatus::Yellow, 1_000, "old")),
                Ok(snapshot(UsageStatus::Red, 2_000, "new")),
            ]),
            successful_sources(),
            fixed_now(9_999),
        );

        service
            .rebuild_from_persisted_at(local_time(1_000), |_| {})
            .await;
        let replaced = service
            .rebuild_from_persisted_at(local_time(2_000), |_| {})
            .await;

        assert_eq!(replaced.status, UsageStatus::Red);
        assert_eq!(replaced.agents.len(), 1);
        assert_eq!(replaced.agents[0].agent_module_id, "new");
        assert!(!serde_json::to_string(&replaced).unwrap().contains("old"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cas_narrow_race_returns_refreshing_without_a_second_collection() {
        let calls = Arc::new(AtomicUsize::new(0));
        let refresh_sources: RefreshSourcesFn = {
            let calls = calls.clone();
            Arc::new(move || {
                calls.fetch_add(1, AtomicOrdering::SeqCst);
                Box::pin(async { Ok(()) })
            })
        };
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            Arc::new(|now| Ok(snapshot(UsageStatus::Green, now.timestamp(), "row"))),
            refresh_sources,
            fixed_now(9_999),
        ));
        let order_guard = service.commit_order.lock().await;
        let published = Arc::new(StdMutex::new(Vec::new()));
        let first = tokio::spawn({
            let service = service.clone();
            let published = published.clone();
            async move {
                service
                    .refresh_at(local_time(3_000), move |snapshot| {
                        published.lock().unwrap().push(snapshot.refresh_in_progress)
                    })
                    .await
            }
        });

        timeout(TEST_TIMEOUT, async {
            while !service.refresh_active.load(AtomicOrdering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first refresh wins CAS");
        let second = timeout(TEST_TIMEOUT, service.refresh_at(local_time(3_001), |_| {}))
            .await
            .expect("blocked refresh returns immediately");
        assert!(second.refresh_in_progress);
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);

        drop(order_guard);
        let final_snapshot = timeout(TEST_TIMEOUT, first)
            .await
            .expect("first refresh completes")
            .unwrap();
        assert!(!final_snapshot.refresh_in_progress);
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(*published.lock().unwrap(), vec![true, false]);
    }

    #[tokio::test]
    async fn publish_callback_never_runs_under_the_cache_lock() {
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            Arc::new(|now| Ok(snapshot(UsageStatus::Green, now.timestamp(), "row"))),
            successful_sources(),
            fixed_now(9_999),
        ));
        let observed = service.clone();

        service
            .refresh_at(local_time(4_000), move |_| {
                assert!(
                    observed.cache.try_read().is_ok(),
                    "publish callback must not hold the cache lock",
                );
            })
            .await;
    }

    #[tokio::test]
    async fn ordinary_rebuild_during_refresh_preserves_active_and_publish_order() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let refresh_sources: RefreshSourcesFn = {
            let started = started.clone();
            let release = release.clone();
            Arc::new(move || {
                let started = started.clone();
                let release = release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(())
                })
            })
        };
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            queue_projector(vec![
                Ok(snapshot(UsageStatus::Yellow, 5_001, "during")),
                Ok(snapshot(UsageStatus::Red, 5_002, "final")),
            ]),
            refresh_sources,
            fixed_now(9_999),
        ));
        let published = Arc::new(StdMutex::new(Vec::new()));
        let started_wait = started.notified();
        let refresh = tokio::spawn({
            let service = service.clone();
            let published = published.clone();
            async move {
                service
                    .refresh_at(local_time(5_002), move |snapshot| {
                        published
                            .lock()
                            .unwrap()
                            .push((snapshot.status, snapshot.refresh_in_progress));
                    })
                    .await
            }
        });
        timeout(TEST_TIMEOUT, started_wait)
            .await
            .expect("source refresh starts");

        let during = service
            .rebuild_from_persisted_at(local_time(5_001), {
                let published = published.clone();
                move |snapshot| {
                    published
                        .lock()
                        .unwrap()
                        .push((snapshot.status, snapshot.refresh_in_progress));
                }
            })
            .await;
        assert_eq!(during.status, UsageStatus::Yellow);
        assert!(during.refresh_in_progress);

        release.notify_one();
        let final_snapshot = timeout(TEST_TIMEOUT, refresh)
            .await
            .expect("refresh completes")
            .unwrap();
        assert_eq!(final_snapshot.status, UsageStatus::Red);
        assert!(!final_snapshot.refresh_in_progress);
        assert_eq!(
            *published.lock().unwrap(),
            vec![
                (UsageStatus::Unknown, true),
                (UsageStatus::Yellow, true),
                (UsageStatus::Red, false),
            ],
        );
    }

    #[tokio::test]
    async fn source_failure_after_external_rebuild_preserves_the_new_current_body() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let refresh_sources: RefreshSourcesFn = {
            let started = started.clone();
            let release = release.clone();
            Arc::new(move || {
                let started = started.clone();
                let release = release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Err(())
                })
            })
        };
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            queue_projector(vec![Ok(snapshot(
                UsageStatus::Yellow,
                6_001,
                "new-current-body",
            ))]),
            refresh_sources,
            fixed_now(9_999),
        ));
        let started_wait = started.notified();
        let refresh = tokio::spawn({
            let service = service.clone();
            async move { service.refresh_at(local_time(6_002), |_| {}).await }
        });
        timeout(TEST_TIMEOUT, started_wait).await.unwrap();
        let rebuilt = service
            .rebuild_from_persisted_at(local_time(6_001), |_| {})
            .await;
        assert!(rebuilt.refresh_in_progress);
        release.notify_one();
        let failed = timeout(TEST_TIMEOUT, refresh).await.unwrap().unwrap();
        assert_eq!(failed.status, UsageStatus::Yellow);
        assert_eq!(failed.agents[0].agent_module_id, "new-current-body");
        assert!(failed.stale);
        assert!(!failed.refresh_in_progress);
    }

    #[tokio::test]
    async fn external_failure_during_manual_refresh_never_clears_the_active_lease() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let refresh_sources: RefreshSourcesFn = {
            let started = started.clone();
            let release = release.clone();
            Arc::new(move || {
                let started = started.clone();
                let release = release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(())
                })
            })
        };
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            queue_projector(vec![Ok(snapshot(
                UsageStatus::Green,
                6_100,
                "manual-final",
            ))]),
            refresh_sources,
            fixed_now(9_999),
        ));
        let published = Arc::new(StdMutex::new(Vec::new()));
        let started_wait = started.notified();
        let manual = tokio::spawn({
            let service = service.clone();
            let published = published.clone();
            async move {
                service
                    .refresh_at(local_time(6_100), move |snapshot| {
                        published
                            .lock()
                            .unwrap()
                            .push((snapshot.stale, snapshot.refresh_in_progress));
                    })
                    .await
            }
        });
        timeout(TEST_TIMEOUT, started_wait).await.unwrap();

        let external_failure = service
            .mark_refresh_failed_at(6_099, {
                let published = published.clone();
                move |snapshot| {
                    published
                        .lock()
                        .unwrap()
                        .push((snapshot.stale, snapshot.refresh_in_progress));
                }
            })
            .await;
        assert!(external_failure.stale);
        assert_eq!(
            external_failure.refresh_error.as_deref(),
            Some("tray_usage_refresh_failed"),
        );
        assert!(external_failure.refresh_in_progress);
        assert!(service.refresh_active.load(AtomicOrdering::Acquire));

        release.notify_one();
        let terminal = timeout(TEST_TIMEOUT, manual).await.unwrap().unwrap();
        assert!(!terminal.stale);
        assert!(!terminal.refresh_in_progress);
        assert!(!service.refresh_active.load(AtomicOrdering::Acquire));
        assert_eq!(
            *published.lock().unwrap(),
            vec![(false, true), (true, true), (false, false)],
        );
    }

    #[tokio::test]
    async fn cancelled_refresh_releases_gate_and_repairs_public_progress() {
        let started = Arc::new(Notify::new());
        let never_release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let refresh_sources: RefreshSourcesFn = {
            let started = started.clone();
            let never_release = never_release.clone();
            let calls = calls.clone();
            Arc::new(move || {
                let call = calls.fetch_add(1, AtomicOrdering::SeqCst);
                let started = started.clone();
                let never_release = never_release.clone();
                Box::pin(async move {
                    if call == 0 {
                        started.notify_one();
                        never_release.notified().await;
                    }
                    Ok(())
                })
            })
        };
        let service = Arc::new(TrayUsageService::new_with_dependencies(
            Arc::new(|now| Ok(snapshot(UsageStatus::Green, now.timestamp(), "row"))),
            refresh_sources,
            fixed_now(9_999),
        ));
        let started_wait = started.notified();
        let first = tokio::spawn({
            let service = service.clone();
            async move { service.refresh_at(local_time(7_000), |_| {}).await }
        });
        timeout(TEST_TIMEOUT, started_wait).await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());

        let cached = service.cached_snapshot().await;
        assert!(!cached.refresh_in_progress);
        let second = timeout(TEST_TIMEOUT, service.refresh_at(local_time(7_001), |_| {}))
            .await
            .expect("second refresh is admitted after cancellation");
        assert!(!second.refresh_in_progress);
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn production_refresh_samples_the_clock_after_source_work() {
        let source_finished = Arc::new(AtomicBool::new(false));
        let refresh_sources: RefreshSourcesFn = {
            let source_finished = source_finished.clone();
            Arc::new(move || {
                let source_finished = source_finished.clone();
                Box::pin(async move {
                    source_finished.store(true, AtomicOrdering::Release);
                    Ok(())
                })
            })
        };
        let projected_at = Arc::new(StdMutex::new(Vec::new()));
        let project_snapshot: ProjectSnapshotFn = {
            let projected_at = projected_at.clone();
            Arc::new(move |now| {
                projected_at.lock().unwrap().push(now.timestamp());
                Ok(snapshot(UsageStatus::Green, now.timestamp(), "row"))
            })
        };
        let post_midnight = local_time(1_800_000_000);
        let now: NowFn = {
            let source_finished = source_finished.clone();
            Arc::new(move || {
                assert!(source_finished.load(AtomicOrdering::Acquire));
                post_midnight
            })
        };
        let service =
            TrayUsageService::new_with_dependencies(project_snapshot, refresh_sources, now);

        let refreshed = service.refresh(|_| {}).await;

        assert_eq!(refreshed.generated_at, post_midnight.timestamp());
        assert_eq!(
            *projected_at.lock().unwrap(),
            vec![post_midnight.timestamp()]
        );
    }

    #[test]
    fn selector_uses_enabled_subscription_quota_and_actual_session_bindings() {
        let providers = vec![
            provider_view("quota", BillingKind::Subscription, true, Some("fake"), &[]),
            provider_view(
                "metered-quota",
                BillingKind::Metered,
                true,
                Some("fake"),
                &[],
            ),
            provider_view(
                "disabled-quota",
                BillingKind::Subscription,
                false,
                Some("fake"),
                &[],
            ),
            provider_view("missing-quota", BillingKind::Subscription, true, None, &[]),
            provider_view(
                "actual-session",
                BillingKind::Metered,
                true,
                None,
                &["codex"],
            ),
            provider_view(
                "disabled-session",
                BillingKind::Metered,
                false,
                None,
                &["claude"],
            ),
            provider_view("capability-only", BillingKind::Metered, true, None, &[]),
            provider_view(
                "both",
                BillingKind::Subscription,
                true,
                Some("fake"),
                &["claude"],
            ),
        ];

        assert_eq!(
            select_refresh_candidates(&providers),
            vec![
                RefreshCandidate {
                    provider_id: "quota".to_string(),
                    quota: true,
                    session: false
                },
                RefreshCandidate {
                    provider_id: "actual-session".to_string(),
                    quota: false,
                    session: true
                },
                RefreshCandidate {
                    provider_id: "both".to_string(),
                    quota: true,
                    session: true
                },
            ],
        );
    }

    #[tokio::test]
    async fn every_source_is_polled_even_when_an_earlier_attempt_waits_or_fails() {
        let first_release = Arc::new(Notify::new());
        let later_started = Arc::new(Notify::new());
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let quota_attempt: QuotaAttemptFn = {
            let first_release = first_release.clone();
            let later_started = later_started.clone();
            let calls = calls.clone();
            Arc::new(move |provider_id| {
                let first_release = first_release.clone();
                let later_started = later_started.clone();
                let calls = calls.clone();
                Box::pin(async move {
                    calls.lock().unwrap().push(format!("quota:{provider_id}"));
                    if provider_id == "first" {
                        first_release.notified().await;
                        Err(AppError::Message("quota-source-sk-secret".to_string()))
                    } else {
                        later_started.notify_one();
                        Ok(())
                    }
                })
            })
        };
        let session_attempt: SessionAttemptFn = {
            let calls = calls.clone();
            Arc::new(move |provider_id| {
                let calls = calls.clone();
                Box::pin(async move {
                    calls.lock().unwrap().push(format!("session:{provider_id}"));
                    Ok(ProviderSessionSyncResult {
                        warnings: vec!["visible warning".to_string()],
                        ..ProviderSessionSyncResult::default()
                    })
                })
            })
        };
        let candidates = vec![
            RefreshCandidate {
                provider_id: "first".to_string(),
                quota: true,
                session: false,
            },
            RefreshCandidate {
                provider_id: "later".to_string(),
                quota: true,
                session: true,
            },
        ];
        let later_wait = later_started.notified();
        let task = tokio::spawn(execute_refresh_candidates(
            candidates,
            quota_attempt,
            session_attempt,
        ));

        timeout(TEST_TIMEOUT, later_wait)
            .await
            .expect("later attempt is polled before first completes");
        first_release.notify_one();
        assert_eq!(timeout(TEST_TIMEOUT, task).await.unwrap().unwrap(), Err(()));
        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&"quota:first".to_string()));
        assert!(calls.contains(&"quota:later".to_string()));
        assert!(calls.contains(&"session:later".to_string()));
    }

    #[tokio::test]
    async fn session_warnings_succeed_but_errors_and_join_failures_do_not_skip_peers() {
        let quota_calls = Arc::new(AtomicUsize::new(0));
        let quota_attempt: QuotaAttemptFn = {
            let quota_calls = quota_calls.clone();
            Arc::new(move |_| {
                quota_calls.fetch_add(1, AtomicOrdering::SeqCst);
                Box::pin(async { Ok(()) })
            })
        };
        let warning_session: SessionAttemptFn = Arc::new(|_| {
            Box::pin(async {
                Ok(ProviderSessionSyncResult {
                    warnings: vec!["warning".to_string()],
                    ..ProviderSessionSyncResult::default()
                })
            })
        });
        let candidates = vec![RefreshCandidate {
            provider_id: "warning".to_string(),
            quota: false,
            session: true,
        }];
        assert_eq!(
            execute_refresh_candidates(candidates, quota_attempt.clone(), warning_session,).await,
            Ok(()),
        );

        let failing_session: SessionAttemptFn = Arc::new(|provider_id| {
            Box::pin(async move {
                if provider_id == "join" {
                    tauri::async_runtime::spawn_blocking(|| -> ProviderSessionSyncResult {
                        panic!("session worker panicked")
                    })
                    .await
                    .map_err(|_| AppError::Message("session join failed".to_string()))
                } else {
                    Ok(ProviderSessionSyncResult {
                        errors: vec!["session-result-sk-secret".to_string()],
                        ..ProviderSessionSyncResult::default()
                    })
                }
            })
        });
        let candidates = vec![
            RefreshCandidate {
                provider_id: "join".to_string(),
                quota: false,
                session: true,
            },
            RefreshCandidate {
                provider_id: "result-error".to_string(),
                quota: false,
                session: true,
            },
            RefreshCandidate {
                provider_id: "quota-peer".to_string(),
                quota: true,
                session: false,
            },
        ];

        assert_eq!(
            execute_refresh_candidates(candidates, quota_attempt, failing_session).await,
            Err(()),
        );
        assert_eq!(quota_calls.load(AtomicOrdering::SeqCst), 1);
    }
}
