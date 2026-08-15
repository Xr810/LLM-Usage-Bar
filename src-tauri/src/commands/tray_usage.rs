use crate::app_state::AppState;
use crate::config::settings::{ApiBudgetConfig, ApiBudgetMode};
use crate::error::AppError;
pub use crate::tray_popover::MainWindowDestination;
use crate::usage::domain::UsageProviderView;
use crate::usage::tray_snapshot::TrayUsageSnapshot;
use std::future::Future;
use std::sync::Arc;
use tauri::{AppHandle, State};

type SnapshotPublisher = Arc<dyn for<'a> Fn(&'a TrayUsageSnapshot) + Send + Sync + 'static>;

#[tauri::command]
pub fn hide_tray_popover(app: AppHandle) -> Result<(), AppError> {
    crate::tray_popover::hide(&app)
}

#[tauri::command]
pub fn open_main_from_tray(
    app: AppHandle,
    destination: MainWindowDestination,
) -> Result<(), AppError> {
    crate::tray_popover::open_main_window(&app, destination)
}

#[tauri::command]
pub fn take_pending_main_window_destination() -> Result<Option<MainWindowDestination>, AppError> {
    crate::tray_popover::take_pending_main_window_destination()
}

/// The renderer has painted the destination it was sent to; the window can be
/// shown now instead of letting the user watch it get there.
#[tauri::command]
pub fn acknowledge_main_window_ready() {
    crate::tray_popover::acknowledge_main_window_ready();
}

#[tauri::command]
pub fn quit_from_tray(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub async fn get_tray_usage_snapshot(
    state: State<'_, AppState>,
) -> Result<TrayUsageSnapshot, AppError> {
    get_tray_usage_snapshot_test_hook(|| state.tray_usage_service.cached_snapshot()).await
}

#[tauri::command]
pub async fn refresh_tray_usage(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TrayUsageSnapshot, AppError> {
    let publisher_app = app.clone();
    let publisher: SnapshotPublisher = Arc::new(move |snapshot| {
        crate::tray_status::publish_tray_usage(&publisher_app, snapshot);
    });
    refresh_tray_usage_test_hook(
        |publish: SnapshotPublisher| {
            state
                .tray_usage_service
                .refresh(move |snapshot: &TrayUsageSnapshot| publish(snapshot))
        },
        publisher,
        crate::usage_events::emit_dashboard_invalidated_only,
    )
    .await
}

#[tauri::command]
pub fn get_api_budget_config() -> ApiBudgetConfig {
    crate::config::settings::get_settings().api_budget_config()
}

#[tauri::command]
pub async fn set_api_budget_config(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: ApiBudgetMode,
    shared_daily_budget_usd: Option<String>,
) -> Result<ApiBudgetConfig, AppError> {
    let canonical = shared_daily_budget_usd
        .as_deref()
        .map(crate::usage::budget_migration::canonicalize_daily_budget)
        .transpose()?;
    let mut settings = crate::config::settings::get_settings();
    settings.api_budget_mode = mode;
    settings.shared_api_daily_budget_usd = canonical;
    crate::config::settings::update_settings(settings)?;

    let publisher_app = app.clone();
    state
        .tray_usage_service
        .rebuild_from_persisted(move |snapshot: &TrayUsageSnapshot| {
            crate::tray_status::publish_tray_usage(&publisher_app, snapshot);
        })
        .await;
    crate::usage_events::emit_dashboard_invalidated_only();
    Ok(crate::config::settings::get_settings().api_budget_config())
}

#[tauri::command]
pub async fn set_provider_daily_budget(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    daily_budget_usd: Option<String>,
) -> Result<UsageProviderView, AppError> {
    let publisher_app = app.clone();
    let publisher: SnapshotPublisher = Arc::new(move |snapshot| {
        crate::tray_status::publish_tray_usage(&publisher_app, snapshot);
    });
    set_provider_daily_budget_test_hook(
        || {
            state
                .db
                .set_provider_daily_budget(&provider_id, daily_budget_usd.as_deref())
        },
        |publish: SnapshotPublisher| {
            state
                .tray_usage_service
                .rebuild_from_persisted(move |snapshot: &TrayUsageSnapshot| publish(snapshot))
        },
        publisher,
        crate::usage_events::emit_dashboard_invalidated_only,
    )
    .await
}

pub(crate) async fn get_tray_usage_snapshot_test_hook<C, Fut>(
    cached_snapshot: C,
) -> Result<TrayUsageSnapshot, AppError>
where
    C: FnOnce() -> Fut,
    Fut: Future<Output = TrayUsageSnapshot>,
{
    Ok(cached_snapshot().await)
}

pub(crate) async fn refresh_tray_usage_test_hook<R, Fut, I>(
    refresh: R,
    publish: SnapshotPublisher,
    invalidate: I,
) -> Result<TrayUsageSnapshot, AppError>
where
    R: FnOnce(SnapshotPublisher) -> Fut,
    Fut: Future<Output = TrayUsageSnapshot>,
    I: FnOnce(),
{
    let snapshot = refresh(publish).await;
    invalidate();
    Ok(snapshot)
}

pub(crate) async fn set_provider_daily_budget_test_hook<M, R, Fut, I>(
    mutate: M,
    rebuild: R,
    publish: SnapshotPublisher,
    invalidate: I,
) -> Result<UsageProviderView, AppError>
where
    M: FnOnce() -> Result<UsageProviderView, AppError>,
    R: FnOnce(SnapshotPublisher) -> Fut,
    Fut: Future<Output = TrayUsageSnapshot>,
    I: FnOnce(),
{
    let provider = mutate()?;
    rebuild(publish).await;
    invalidate();
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;
    use crate::usage::status::UsageStatus;
    use crate::usage::system_providers::OPENAI_API_ID;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    type TestPublisher = SnapshotPublisher;

    fn snapshot(status: UsageStatus, generated_at: i64) -> TrayUsageSnapshot {
        let mut snapshot = TrayUsageSnapshot::unknown(generated_at);
        snapshot.status = status;
        snapshot
    }

    #[tokio::test]
    async fn get_is_cache_only() {
        let cache_reads = Arc::new(AtomicUsize::new(0));
        let cached = snapshot(UsageStatus::Yellow, 10);
        let result = get_tray_usage_snapshot_test_hook({
            let cache_reads = cache_reads.clone();
            let cached = cached.clone();
            move || async move {
                cache_reads.fetch_add(1, Ordering::SeqCst);
                cached
            }
        })
        .await
        .unwrap();

        assert_eq!(result, cached);
        assert_eq!(cache_reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_forwards_publications_and_invalidates_after_completion() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let terminal = snapshot(UsageStatus::Green, 12);
        let publisher: TestPublisher = {
            let calls = calls.clone();
            Arc::new(move |snapshot: &TrayUsageSnapshot| {
                calls
                    .lock()
                    .unwrap()
                    .push(format!("publish:{:?}", snapshot.status));
            })
        };
        let result = refresh_tray_usage_test_hook(
            {
                let calls = calls.clone();
                let terminal = terminal.clone();
                move |publish: TestPublisher| async move {
                    calls.lock().unwrap().push("refresh-start".to_string());
                    publish(&snapshot(UsageStatus::Unknown, 11));
                    publish(&terminal);
                    calls.lock().unwrap().push("refresh-end".to_string());
                    terminal
                }
            },
            publisher,
            {
                let calls = calls.clone();
                move || calls.lock().unwrap().push("invalidate".to_string())
            },
        )
        .await
        .unwrap();

        assert_eq!(result, terminal);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                "refresh-start",
                "publish:Unknown",
                "publish:Green",
                "refresh-end",
                "invalidate",
            ]
        );
    }

    #[tokio::test]
    async fn blocked_refresh_returns_cache_without_a_second_publication() {
        let publications = Arc::new(AtomicUsize::new(0));
        let invalidations = Arc::new(AtomicUsize::new(0));
        let cached = snapshot(UsageStatus::Red, 20);
        let publisher: TestPublisher = {
            let publications = publications.clone();
            Arc::new(move |_snapshot: &TrayUsageSnapshot| {
                publications.fetch_add(1, Ordering::SeqCst);
            })
        };
        let result = refresh_tray_usage_test_hook(
            {
                let cached = cached.clone();
                move |_publish: TestPublisher| async move { cached }
            },
            publisher,
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(result, cached);
        assert_eq!(publications.load(Ordering::SeqCst), 0);
        assert_eq!(invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn budget_mutates_rebuilds_publishes_and_invalidates_once() {
        let db = Arc::new(Database::memory().unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE usage_providers SET quota_config = ?2 WHERE id = ?1",
                rusqlite::params![OPENAI_API_ID, r#"{"apiKey":"budget-secret-sentinel"}"#],
            )
            .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let projected = snapshot(UsageStatus::Yellow, 30);
        let publisher: TestPublisher = {
            let calls = calls.clone();
            Arc::new(move |snapshot: &TrayUsageSnapshot| {
                calls
                    .lock()
                    .unwrap()
                    .push(format!("publish:{:?}", snapshot.status));
            })
        };
        let view = set_provider_daily_budget_test_hook(
            {
                let db = db.clone();
                let calls = calls.clone();
                move || {
                    calls.lock().unwrap().push("mutate".to_string());
                    db.set_provider_daily_budget(OPENAI_API_ID, Some("25.500"))
                }
            },
            {
                let calls = calls.clone();
                let projected = projected.clone();
                move |publish: TestPublisher| async move {
                    calls.lock().unwrap().push("rebuild".to_string());
                    publish(&projected);
                    projected
                }
            },
            publisher,
            {
                let calls = calls.clone();
                move || calls.lock().unwrap().push("invalidate".to_string())
            },
        )
        .await
        .unwrap();

        assert_eq!(view.daily_budget_usd.as_deref(), Some("25.5"));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["mutate", "rebuild", "publish:Yellow", "invalidate"]
        );
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("quota_config"));
        assert!(!serialized.contains("budget-secret-sentinel"));
    }

    #[tokio::test]
    async fn budget_mutation_error_skips_rebuild_publish_and_invalidation() {
        let rebuilds = Arc::new(AtomicUsize::new(0));
        let publications = Arc::new(AtomicUsize::new(0));
        let invalidations = Arc::new(AtomicUsize::new(0));
        let publisher: TestPublisher = {
            let publications = publications.clone();
            Arc::new(move |_snapshot: &TrayUsageSnapshot| {
                publications.fetch_add(1, Ordering::SeqCst);
            })
        };
        let result = set_provider_daily_budget_test_hook(
            || Err(AppError::Message("invalid_daily_budget".to_string())),
            {
                let rebuilds = rebuilds.clone();
                move |_publish: TestPublisher| async move {
                    rebuilds.fetch_add(1, Ordering::SeqCst);
                    TrayUsageSnapshot::unknown(0)
                }
            },
            publisher,
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(rebuilds.load(Ordering::SeqCst), 0);
        assert_eq!(publications.load(Ordering::SeqCst), 0);
        assert_eq!(invalidations.load(Ordering::SeqCst), 0);
    }
}
