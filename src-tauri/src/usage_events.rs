//! 使用统计实时刷新事件模块
//!
//! 当 `proxy_request_logs` 表写入新数据时（代理日志、会话同步、归档等），
//! 通过本模块向前端 emit `usage-log-recorded` 事件，让 UsageDashboard
//! 立刻 invalidate 查询缓存而无需等待轮询周期。
//!
//! 设计要点：
//! - 全局单例 AppHandle：写日志路径上不持有 AppHandle，用 OnceCell 共享。
//! - 200ms 防抖合并：流式响应等场景在短时间内可能写入多条日志，
//!   合并成一次事件可避免前端连续 invalidate。
//! - 不阻塞写入：通知失败仅记录 warn 日志，不向上传播错误。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

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

/// 防抖标记：true 表示已有调度任务在等待 emit，后续通知合并到该任务。
static LOG_EMIT_SCHEDULED: AtomicBool = AtomicBool::new(false);
static DASHBOARD_EMIT_SCHEDULED: AtomicBool = AtomicBool::new(false);

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
/// 内部 200ms 防抖合并，绝不阻塞调用线程。
pub fn notify_log_recorded() {
    // AppHandle 未注入（典型出现在单元测试或 setup 之前）：直接放弃。
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };

    schedule_debounced_unit_event(
        handle.clone(),
        EVENT_USAGE_DASHBOARD_INVALIDATED,
        &DASHBOARD_EMIT_SCHEDULED,
    );
    schedule_debounced_unit_event(
        handle.clone(),
        EVENT_USAGE_LOG_RECORDED,
        &LOG_EMIT_SCHEDULED,
    );
}

fn schedule_debounced_unit_event(
    handle: AppHandle,
    event_name: &'static str,
    scheduled: &'static AtomicBool,
) {
    if scheduled.swap(true, Ordering::AcqRel) {
        return;
    }

    std::thread::spawn(move || {
        std::thread::sleep(DEBOUNCE_WINDOW);
        scheduled.store(false, Ordering::Release);

        if let Err(error) = handle.emit(event_name, ()) {
            log::warn!("emit {event_name} 失败: {error}");
        }
    });
}

/// Notify every Agent dashboard immediately after a successful metadata or
/// protected-credential mutation. The payload is deliberately unit so no
/// binding, Provider, or credential-derived identifier can escape in events.
pub fn notify_dashboard_invalidated() {
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };
    if let Err(error) = handle.emit(EVENT_USAGE_DASHBOARD_INVALIDATED, ()) {
        log::warn!("emit {EVENT_USAGE_DASHBOARD_INVALIDATED} 失败: {error}");
    }
}

#[cfg(test)]
fn dashboard_invalidation_payload() {}

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
        dashboard_invalidation_payload, ingestion_error_payload, DASHBOARD_EMIT_SCHEDULED,
        EVENT_USAGE_DASHBOARD_INVALIDATED, LOG_EMIT_SCHEDULED,
    };

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
        assert_eq!(
            serde_json::to_value(dashboard_invalidation_payload()).unwrap(),
            serde_json::Value::Null
        );
    }

    #[test]
    fn usage_writes_schedule_dashboard_invalidation_on_an_independent_debounce_flag() {
        let source = include_str!("usage_events.rs");
        let scheduling = [
            "EVENT_USAGE_DASHBOARD_",
            "INVALIDATED,\n        &DASHBOARD_EMIT_SCHEDULED,",
        ]
        .concat();
        assert!(source.contains(&scheduling));
        assert!(!std::ptr::eq(
            &LOG_EMIT_SCHEDULED,
            &DASHBOARD_EMIT_SCHEDULED
        ));
    }
}
