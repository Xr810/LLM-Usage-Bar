//! BE-21: read-only `get_guard_state` (contract C12).
//!
//! Community builds never return `capped` or `killed`. Counters stay at 0
//! until BE-15 adds guard storage. This batch does not split "today" at local
//! midnight. This command does not suspend, kill, or grant an allowance.
//!
//! `guard-state-changed` is not emitted from the getter. It is emitted from
//! tray publication only when the projected view changes.

use crate::app_state::AppState;
use crate::error::AppError;
use crate::model::UsageStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, State};

pub const GUARD_STATE_CHANGED_EVENT: &str = "guard-state-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardLevel {
    Ok,
    Warn,
    Capped,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedToday {
    pub count: u32,
    pub usd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardStateView {
    pub state: GuardLevel,
    pub blocked_today: u32,
    pub allowed_today: AllowedToday,
    pub pending_count: usize,
    pub pending: Vec<crate::policy::PendingMeta>,
}

static LAST_PUBLISHED_GUARD: Mutex<Option<GuardStateView>> = Mutex::new(None);

/// Interim community warn source: yellow or red API-budget status is `warn`.
/// Green and unknown are `ok`. `capped` and `killed` are not representable here.
///
/// TODO(BE-20): after BE-20 merges, derive C12 budget warn from C10 `level`
/// (`level != ok` => warn). Pacing colour is display-only.
pub fn guard_state_for_budget(status: UsageStatus) -> GuardStateView {
    let state = match status {
        UsageStatus::Yellow | UsageStatus::Red => GuardLevel::Warn,
        UsageStatus::Green | UsageStatus::Unknown => GuardLevel::Ok,
    };
    GuardStateView {
        state,
        blocked_today: 0,
        pending_count: 0,
        pending: Vec::new(),
        allowed_today: AllowedToday {
            count: 0,
            usd: "0".to_string(),
        },
    }
}

pub fn guard_state_changed_payload(view: &GuardStateView) -> Value {
    serde_json::to_value(view).unwrap_or_else(|_| Value::Null)
}

/// `Some` when `status` projects a view different from the last publication.
fn next_guard_publication(
    previous: Option<&GuardStateView>,
    status: UsageStatus,
) -> Option<GuardStateView> {
    let mut next = guard_state_for_budget(status);
    if let Some(previous) = previous {
        next.pending = previous.pending.clone();
        next.pending_count = next.pending.len();
    }
    (previous != Some(&next)).then_some(next)
}

/// Emit `guard-state-changed` only when the projected view changes.
pub fn publish_guard_state_if_changed<R: tauri::Runtime>(app: &AppHandle<R>, status: UsageStatus) {
    let mut last = LAST_PUBLISHED_GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(view) = next_guard_publication(last.as_ref(), status) else {
        return;
    };
    let _ = app.emit(
        GUARD_STATE_CHANGED_EVENT,
        guard_state_changed_payload(&view),
    );
    *last = Some(view);
}

pub fn publish_pending_state_if_changed<R: tauri::Runtime>(
    app: &AppHandle<R>,
    pending: Vec<crate::policy::PendingMeta>,
) {
    let mut last = LAST_PUBLISHED_GUARD
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut view = last
        .clone()
        .unwrap_or_else(|| guard_state_for_budget(UsageStatus::Unknown));
    view.pending_count = pending.len();
    view.pending = pending;
    if last.as_ref() == Some(&view) {
        return;
    }
    let _ = app.emit(
        GUARD_STATE_CHANGED_EVENT,
        guard_state_changed_payload(&view),
    );
    *last = Some(view);
}

#[tauri::command]
pub async fn get_guard_state(state: State<'_, AppState>) -> Result<GuardStateView, AppError> {
    let snapshot = state.tray_usage_service.cached_snapshot().await;
    let mut view = guard_state_for_budget(snapshot.api_budget.status);
    view.pending = state.pending_registry.view();
    view.pending_count = view.pending.len();
    Ok(view)
}

#[tauri::command]
pub fn guard_allow_once(
    state: State<'_, AppState>,
    pending_id: Option<String>,
) -> crate::policy::ReleaseResult {
    state.pending_registry.release(pending_id.as_deref())
}

#[tauri::command]
pub fn guard_allow_more_today(
    state: State<'_, AppState>,
    amount_usd: Option<crate::config::settings::AllowanceUsd>,
) -> crate::policy::AllowanceResult {
    state
        .pending_registry
        .allow_more_today(amount_usd.map(|amount| amount.as_str()))
}

#[tauri::command]
pub fn guard_deny_pending(
    state: State<'_, AppState>,
    pending_id: Option<String>,
) -> crate::policy::DenyResult {
    state.pending_registry.deny(pending_id.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_events_include_full_view_skip_duplicates_and_preserve_pending_on_budget_change() {
        use std::sync::Arc;
        use tauri::Listener;
        let app = tauri::test::mock_app();
        *LAST_PUBLISHED_GUARD.lock().unwrap() = None;
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = events.clone();
        app.listen(GUARD_STATE_CHANGED_EVENT, move |event| {
            sink.lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        let pending = vec![crate::policy::testing::meta("event", 1, "scope")];
        publish_pending_state_if_changed(app.handle(), pending.clone());
        publish_pending_state_if_changed(app.handle(), pending);
        publish_guard_state_if_changed(app.handle(), UsageStatus::Yellow);
        publish_pending_state_if_changed(app.handle(), Vec::new());
        publish_pending_state_if_changed(app.handle(), Vec::new());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["pendingCount"], 1);
        assert_eq!(events[1]["pendingCount"], 1);
        assert_eq!(events[1]["state"], "warn");
        assert_eq!(events[2]["pendingCount"], 0);
        assert_eq!(events[2]["pending"], json!([]));
    }

    #[tokio::test]
    async fn all_command_shells_delegate_and_community_is_noop() {
        use crate::policy::{
            testing::{meta, TestRegistry},
            PendingRegistry,
        };
        use std::{sync::Arc, time::Duration};
        use tauri::Manager;
        for injected in [false, true] {
            let db = Arc::new(crate::Database::memory().unwrap());
            let registry = Arc::new(TestRegistry::new(4));
            let mut injections = crate::Injections::community();
            if injected {
                let registry = registry.clone();
                injections.pending_registry = Box::new(move |_| Ok(registry));
            }
            let state = AppState::new_with_injections(
                db.clone(),
                crate::secrets::unavailable_credential_store(),
                Arc::new(crate::quota::QuotaService::new(db)),
                injections,
            );
            let app = tauri::test::mock_app();
            app.manage(state);
            let mut tickets = Vec::new();
            if injected {
                for (id, time, scope) in [("late", 20, "A"), ("head", 10, "B"), ("same", 30, "A")] {
                    tickets.push(
                        registry
                            .enqueue(meta(id, time, scope), Duration::from_secs(60))
                            .unwrap(),
                    );
                }
            }
            let result = guard_allow_once(app.state(), None);
            assert_eq!(result.released, injected);
            assert_eq!(
                result.pending_id.as_deref(),
                if injected { Some("head") } else { None }
            );
            assert!(!guard_allow_once(app.state(), Some("missing".into())).released);
            let result = guard_allow_more_today(
                app.state(),
                Some(crate::config::settings::AllowanceUsd::Ten),
            );
            assert_eq!(result.released_count, if injected { 2 } else { 0 });
            assert_eq!(result.allowance_usd, if injected { "10" } else { "0" });
            if injected {
                tickets.push(
                    registry
                        .enqueue(meta("deny-id", 40, "A"), Duration::from_secs(60))
                        .unwrap(),
                );
                tickets.push(
                    registry
                        .enqueue(meta("deny-all", 50, "B"), Duration::from_secs(60))
                        .unwrap(),
                );
            }
            assert_eq!(
                guard_deny_pending(app.state(), Some("deny-id".into())).denied_count,
                usize::from(injected)
            );
            assert_eq!(
                guard_deny_pending(app.state(), None).denied_count,
                usize::from(injected)
            );
            assert_eq!(guard_deny_pending(app.state(), None).denied_count, 0);
            let view = get_guard_state(app.state()).await.unwrap();
            assert_eq!(view.pending_count, 0);
            assert!(view.pending.is_empty());
        }
    }

    #[test]
    fn community_guard_is_only_ok_or_warn_with_zero_counts() {
        for status in [
            UsageStatus::Green,
            UsageStatus::Unknown,
            UsageStatus::Yellow,
            UsageStatus::Red,
        ] {
            let view = guard_state_for_budget(status);
            let payload = guard_state_changed_payload(&view);
            assert_eq!(payload, serde_json::to_value(&view).unwrap());
            assert!(payload.get("diff").is_none());
            assert!(payload.get("killScope").is_none());
            assert!(payload.get("cappedRule").is_none());
            assert!(payload.get("killedAt").is_none());
            assert!(payload.get("resumeAt").is_none());
            assert_eq!(payload["blockedToday"], 0);
            assert_eq!(payload["pendingCount"], 0);
            assert_eq!(payload["pending"], json!([]));
            assert_eq!(payload["allowedToday"], json!({"count": 0, "usd": "0"}));
            match status {
                UsageStatus::Yellow | UsageStatus::Red => {
                    assert_eq!(view.state, GuardLevel::Warn);
                }
                UsageStatus::Green | UsageStatus::Unknown => {
                    assert_eq!(view.state, GuardLevel::Ok);
                }
            }
            let state = payload["state"].as_str().unwrap();
            assert!(state == "ok" || state == "warn", "{state}");
        }
    }

    #[test]
    fn guard_event_is_emitted_only_when_the_projected_view_changes() {
        let ok = guard_state_for_budget(UsageStatus::Green);
        assert!(next_guard_publication(None, UsageStatus::Green).is_some());
        assert!(next_guard_publication(Some(&ok), UsageStatus::Unknown).is_none());
        assert!(next_guard_publication(Some(&ok), UsageStatus::Yellow).is_some());
        let warn = guard_state_for_budget(UsageStatus::Yellow);
        assert!(next_guard_publication(Some(&warn), UsageStatus::Red).is_none());
    }
}
