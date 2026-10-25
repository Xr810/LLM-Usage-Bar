//! BE-21: read-only `get_guard_state` (contract C12).
//!
//! Community builds never return `capped` or `killed`. Counters stay at 0
//! until BE-15 adds guard storage. "Today" for those counters is the user
//! local midnight, the same boundary `TrayUsageWindows` already uses. This
//! command does not suspend, kill, or grant an allowance.

use crate::app_state::AppState;
use crate::error::AppError;
use crate::model::UsageStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// Yellow or red API-budget status is `warn`. Green and unknown are `ok`.
/// `capped` and `killed` are not representable here.
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

#[tauri::command]
pub async fn get_guard_state(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<GuardStateView, AppError> {
    let snapshot = state.tray_usage_service.cached_snapshot().await;
    let view = guard_state_for_budget(snapshot.api_budget.status);
    let _ = app.emit(
        GUARD_STATE_CHANGED_EVENT,
        guard_state_changed_payload(&view),
    );
    Ok(view)
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
}
