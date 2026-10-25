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
    let next = guard_state_for_budget(status);
    (previous != Some(&next)).then_some(next)
}

/// Emit `guard-state-changed` only when the projected view changes.
pub fn publish_guard_state_if_changed(app: &AppHandle, status: UsageStatus) {
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

#[tauri::command]
pub async fn get_guard_state(state: State<'_, AppState>) -> Result<GuardStateView, AppError> {
    let snapshot = state.tray_usage_service.cached_snapshot().await;
    Ok(guard_state_for_budget(snapshot.api_budget.status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
