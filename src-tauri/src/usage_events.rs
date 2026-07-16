//! 使用统计实时刷新事件模块
//!
//! 当 `proxy_request_logs` 表写入新数据时（代理日志、会话同步、归档等），
//! 通过本模块向前端 emit `usage-log-recorded` 事件，让 UsageDashboard
//! 立刻 invalidate 查询缓存，并合并触发托盘快照重建。
//!
//! 设计要点：
//! - 全局单例 AppHandle：写日志路径上不持有 AppHandle，用 OnceCell 共享。
//! - 200ms 防抖合并：流式响应等场景在短时间内可能写入多条日志，
//!   合并成一次事件可避免前端连续 invalidate。
//! - 不阻塞写入：通知失败仅记录 warn 日志，不向上传播错误。

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use serde::Serialize;

/// 前端监听的事件名
pub const EVENT_USAGE_LOG_RECORDED: &str = "usage-log-recorded";
pub const EVENT_USAGE_INGESTION_ERROR: &str = "usage-ingestion-error";
pub const EVENT_USAGE_DASHBOARD_INVALIDATED: &str = "usage-dashboard-invalidated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageIngestionErrorPayload {
    provider_id: String,
    request_id: String,
    message: &'static str,
}

/// 防抖窗口：合并 200ms 内的多次通知。
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

#[derive(Clone, Default)]
struct DebounceGate {
    scheduled: Arc<AtomicBool>,
}

impl DebounceGate {
    fn try_admit(&self) -> bool {
        self.scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn release(&self) {
        self.scheduled.store(false, Ordering::Release);
    }
}

/// Three independent gates retain the existing unit-event debounce behavior
/// while allowing tray projection to coalesce on its own boundary.
static LOG_EMIT_SCHEDULED: LazyLock<DebounceGate> = LazyLock::new(DebounceGate::default);
static DASHBOARD_EMIT_SCHEDULED: LazyLock<DebounceGate> = LazyLock::new(DebounceGate::default);
static TRAY_REBUILD_SCHEDULED: LazyLock<DebounceGate> = LazyLock::new(DebounceGate::default);

/// 在应用 setup 阶段调用一次，注入 AppHandle。
///
/// 重复调用是无害的（OnceLock 仅首次写入生效），但应用启动期只该被
/// `lib.rs::run` 调一次。
pub fn init(handle: AppHandle) {
    if APP_HANDLE.set(handle).is_err() {
        log::debug!("usage_events::init 重复调用，已忽略");
    } else {
        log::info!("[usage-event] AppHandle 已注入，事件推送启用");
    }
}

/// 通知前端有新的使用日志写入。
///
/// 调用方**不**需要持有 AppHandle，可以从任意线程/任意写入路径调用。
/// 两种既有 unit event 与托盘重建分别使用独立的 200ms 防抖门，绝不阻塞
/// 调用线程。
pub fn notify_log_recorded() {
    // AppHandle 未注入（典型出现在单元测试或 setup 之前）：直接放弃。
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };

    let dashboard_handle = handle.clone();
    let log_handle = handle.clone();
    notify_log_recorded_with(
        move || {
            schedule_debounced_unit_event(
                dashboard_handle,
                EVENT_USAGE_DASHBOARD_INVALIDATED,
                &DASHBOARD_EMIT_SCHEDULED,
            );
        },
        move || {
            schedule_debounced_unit_event(
                log_handle,
                EVENT_USAGE_LOG_RECORDED,
                &LOG_EMIT_SCHEDULED,
            );
        },
        schedule_tray_snapshot_rebuild,
    );
}

fn notify_log_recorded_with<Dashboard, Log, Tray>(
    schedule_dashboard: Dashboard,
    schedule_log: Log,
    schedule_tray: Tray,
) where
    Dashboard: FnOnce(),
    Log: FnOnce(),
    Tray: FnOnce(),
{
    schedule_dashboard();
    schedule_log();
    schedule_tray();
}

fn schedule_debounced_action<Spawn>(gate: &DebounceGate, spawn: Spawn)
where
    Spawn: FnOnce(DebounceGate),
{
    if gate.try_admit() {
        spawn(gate.clone());
    }
}

async fn run_debounced_action<Wait, WaitFuture, Action, ActionFuture>(
    gate: DebounceGate,
    wait: Wait,
    action: Action,
) where
    Wait: FnOnce() -> WaitFuture,
    WaitFuture: Future<Output = ()>,
    Action: FnOnce() -> ActionFuture,
    ActionFuture: Future<Output = ()>,
{
    wait().await;
    // Release before beginning the action so a mutation arriving during a
    // potentially slow projection can schedule a follow-up instead of being
    // lost behind the just-finished debounce window.
    gate.release();
    action().await;
}

fn schedule_debounced_unit_event(
    handle: AppHandle,
    event_name: &'static str,
    scheduled: &DebounceGate,
) {
    schedule_debounced_action(scheduled, move |gate| {
        tauri::async_runtime::spawn(run_debounced_action(
            gate,
            || async {
                tokio::time::sleep(DEBOUNCE_WINDOW).await;
            },
            move || async move {
                if handle.emit(event_name, ()).is_err() {
                    log::warn!("usage event emit failed");
                }
            },
        ));
    });
}

fn schedule_tray_snapshot_rebuild() {
    let Some(handle) = APP_HANDLE.get().cloned() else {
        return;
    };

    schedule_debounced_action(&TRAY_REBUILD_SCHEDULED, move |gate| {
        tauri::async_runtime::spawn(run_debounced_action(
            gate,
            || async {
                tokio::time::sleep(DEBOUNCE_WINDOW).await;
            },
            move || async move {
                let service = {
                    let Some(state) = handle.try_state::<crate::store::AppState>() else {
                        log::warn!("tray usage state is unavailable");
                        return;
                    };
                    state.tray_usage_service.clone()
                };
                let _ = service
                    .rebuild_from_persisted(|snapshot| {
                        crate::tray_status::publish_tray_usage(&handle, snapshot);
                    })
                    .await;
            },
        ));
    });
}

/// Emit only the immediate unit dashboard event after a caller already rebuilt
/// and published tray state directly. This deliberately does not schedule a
/// second projection.
pub(crate) fn emit_dashboard_invalidated_only() {
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };
    emit_dashboard_invalidated_only_with(|| {
        if handle.emit(EVENT_USAGE_DASHBOARD_INVALIDATED, ()).is_err() {
            log::warn!("usage dashboard invalidation emit failed");
        }
    });
}

fn emit_dashboard_invalidated_only_with<Emit>(emit: Emit)
where
    Emit: FnOnce(),
{
    emit();
}

/// Notify every Agent dashboard immediately after a successful metadata or
/// protected-credential mutation, then debounce one persisted tray rebuild.
/// The emitted payload remains unit so identifiers and credential-derived data
/// cannot escape through this boundary.
pub fn notify_dashboard_invalidated() {
    notify_dashboard_invalidated_with(
        emit_dashboard_invalidated_only,
        schedule_tray_snapshot_rebuild,
    );
}

fn notify_dashboard_invalidated_with<Emit, Schedule>(emit: Emit, schedule: Schedule)
where
    Emit: FnOnce(),
    Schedule: FnOnce(),
{
    emit();
    schedule();
}

/// Emit a diagnostic-only ingestion failure event without exposing credentials,
/// upstream payloads, or raw database errors to the renderer.
pub fn notify_ingestion_error(provider_id: &str, request_id: &str) {
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };
    let payload = ingestion_error_payload(provider_id, request_id);
    if let Err(error) = handle.emit(EVENT_USAGE_INGESTION_ERROR, payload) {
        log::warn!("emit {EVENT_USAGE_INGESTION_ERROR} 失败: {error}");
    }
}

fn ingestion_error_payload(provider_id: &str, request_id: &str) -> UsageIngestionErrorPayload {
    let _ = (provider_id, request_id);
    UsageIngestionErrorPayload {
        provider_id: "redacted".to_string(),
        request_id: "redacted".to_string(),
        message: "usage ingestion failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        emit_dashboard_invalidated_only_with, ingestion_error_payload,
        notify_dashboard_invalidated_with, notify_log_recorded_with, run_debounced_action,
        schedule_debounced_action, DebounceGate, EVENT_USAGE_DASHBOARD_INVALIDATED,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    #[test]
    fn ingestion_error_payload_redacts_identifiers_and_raw_message() {
        let payload =
            ingestion_error_payload("global-provider-secret", "request-with-api-key-sk-secret");
        assert_eq!(payload.provider_id, "redacted");
        assert_eq!(payload.request_id, "redacted");
        assert_eq!(payload.message, "usage ingestion failed");

        let serialized = serde_json::to_string(&payload).unwrap();
        assert!(!serialized.contains("global-provider-secret"));
        assert!(!serialized.contains("api-key"));
        assert!(!serialized.contains("cret"));
    }

    #[test]
    fn dashboard_invalidation_event_uses_the_global_name_and_unit_payload() {
        assert_eq!(
            EVENT_USAGE_DASHBOARD_INVALIDATED,
            "usage-dashboard-invalidated"
        );
        assert_eq!(serde_json::to_value(()).unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn event_only_invalidation_emits_without_scheduling_a_rebuild() {
        let emitted = AtomicUsize::new(0);
        let scheduled = AtomicUsize::new(0);

        emit_dashboard_invalidated_only_with(|| {
            emitted.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(emitted.load(Ordering::SeqCst), 1);
        assert_eq!(scheduled.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn scheduled_invalidation_emits_each_time_but_debounces_tray_rebuilds() {
        let emitted = AtomicUsize::new(0);
        let admitted = AtomicUsize::new(0);
        let tray_gate = DebounceGate::default();

        for _ in 0..8 {
            notify_dashboard_invalidated_with(
                || {
                    emitted.fetch_add(1, Ordering::SeqCst);
                },
                || {
                    schedule_debounced_action(&tray_gate, |_| {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    });
                },
            );
        }

        assert_eq!(emitted.load(Ordering::SeqCst), 8);
        assert_eq!(admitted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ingestion_burst_keeps_log_dashboard_and_tray_debounce_independent() {
        let log_gate = DebounceGate::default();
        let dashboard_gate = DebounceGate::default();
        let tray_gate = DebounceGate::default();
        let log_admitted = AtomicUsize::new(0);
        let dashboard_admitted = AtomicUsize::new(0);
        let tray_admitted = AtomicUsize::new(0);

        for _ in 0..8 {
            notify_log_recorded_with(
                || {
                    schedule_debounced_action(&dashboard_gate, |_| {
                        dashboard_admitted.fetch_add(1, Ordering::SeqCst);
                    });
                },
                || {
                    schedule_debounced_action(&log_gate, |_| {
                        log_admitted.fetch_add(1, Ordering::SeqCst);
                    });
                },
                || {
                    schedule_debounced_action(&tray_gate, |_| {
                        tray_admitted.fetch_add(1, Ordering::SeqCst);
                    });
                },
            );
        }

        assert_eq!(dashboard_admitted.load(Ordering::SeqCst), 1);
        assert_eq!(log_admitted.load(Ordering::SeqCst), 1);
        assert_eq!(tray_admitted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tray_gate_reopens_before_rebuild_so_a_follow_up_is_not_lost() {
        let gate = DebounceGate::default();
        assert!(gate.try_admit());

        let release_wait = Arc::new(Notify::new());
        let action_started = Arc::new(Notify::new());
        let finish_action = Arc::new(Notify::new());
        let task = tokio::spawn(run_debounced_action(
            gate.clone(),
            {
                let release_wait = release_wait.clone();
                move || async move {
                    release_wait.notified().await;
                }
            },
            {
                let action_started = action_started.clone();
                let finish_action = finish_action.clone();
                move || async move {
                    action_started.notify_one();
                    finish_action.notified().await;
                }
            },
        ));

        let premature_follow_up = AtomicUsize::new(0);
        schedule_debounced_action(&gate, |_| {
            premature_follow_up.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(premature_follow_up.load(Ordering::SeqCst), 0);

        release_wait.notify_one();
        timeout(Duration::from_secs(1), action_started.notified())
            .await
            .expect("rebuild action should start after the injected wait");

        let follow_up = AtomicUsize::new(0);
        schedule_debounced_action(&gate, |_| {
            follow_up.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(follow_up.load(Ordering::SeqCst), 1);

        finish_action.notify_one();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("debounced action should finish")
            .expect("debounced action task should not panic");
    }
}
