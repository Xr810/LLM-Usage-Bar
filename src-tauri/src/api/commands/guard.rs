//! BE-21: read-only `get_guard_state` (contract C12).
//!
//! Community builds retain budget ok/warn and zero counters. Injected providers
//! own enforcement snapshots; the host only joins them to pending metadata.
//!
//! `guard-state-changed` is not emitted from the getter. It is emitted from
//! provider, registry or tray invalidation only when the complete view changes.

use crate::app_state::AppState;
use crate::error::AppError;
use crate::model::UsageStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub use crate::policy::{AllowedToday, GuardLevel};
use crate::policy::{GuardStateNotifier, GuardStateProvider, PendingRegistry};
use tauri::{AppHandle, Emitter, Manager, State};

pub const GUARD_STATE_CHANGED_EVENT: &str = "guard-state-changed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardStateView {
    pub state: GuardLevel,
    pub blocked_today: u32,
    pub allowed_today: AllowedToday,
    pub pending_count: usize,
    pub pending: Vec<crate::policy::PendingMeta>,
}

/// The single command/event projection. Neither source supplies a partial event.
pub fn assemble_guard_state(
    provider: &dyn GuardStateProvider,
    registry: &dyn PendingRegistry,
) -> GuardStateView {
    let core = provider.snapshot();
    let pending = registry.view();
    GuardStateView {
        state: core.state,
        blocked_today: core.blocked_today,
        allowed_today: core.allowed_today,
        pending_count: pending.len(),
        pending,
    }
}

pub fn guard_state_changed_payload(view: &GuardStateView) -> Value {
    serde_json::to_value(view).unwrap_or_else(|_| Value::Null)
}

/// Weak snapshot owners avoid provider/registry -> notifier -> owner cycles.
/// This also works during setup, before AppState is managed by Tauri.
pub fn guard_state_notifier<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> GuardStateNotifier {
    let provider = Arc::downgrade(&state.guard_state);
    let registry = Arc::downgrade(&state.pending_registry);
    let last = state.last_published_guard.clone();
    let app = app.clone();
    GuardStateNotifier::new(move || {
        let (Some(provider), Some(registry)) = (provider.upgrade(), registry.upgrade()) else {
            return;
        };
        // Serialize rebuilding as well as emission, so racing notifications cannot
        // publish an older snapshot after a newer one.
        let mut last = last.lock().unwrap_or_else(|p| p.into_inner());
        let view = assemble_guard_state(provider.as_ref(), registry.as_ref());
        if last.as_ref() != Some(&view) {
            let _ = app.emit(
                GUARD_STATE_CHANGED_EVENT,
                guard_state_changed_payload(&view),
            );
            *last = Some(view);
        }
    })
}

pub fn publish_guard_state_if_changed<R: tauri::Runtime>(app: &AppHandle<R>, status: UsageStatus) {
    if let Some(state) = app.try_state::<AppState>() {
        state.guard_state.update_budget(status);
        guard_state_notifier(app, &state).notify();
    }
}

#[tauri::command]
pub async fn get_guard_state(state: State<'_, AppState>) -> Result<GuardStateView, AppError> {
    let snapshot = state.tray_usage_service.cached_snapshot().await;
    state.guard_state.update_budget(snapshot.api_budget.status);
    Ok(assemble_guard_state(
        state.guard_state.as_ref(),
        state.pending_registry.as_ref(),
    ))
}

#[tauri::command]
pub fn guard_get_upgrade_notice(
    state: State<'_, AppState>,
) -> Option<crate::policy::UpgradeNotice> {
    state.guard_state.upgrade_notice()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardChangeResult {
    pub changed: bool,
}

#[tauri::command]
pub fn guard_ack_upgrade_notice<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    notice_id: String,
) -> GuardChangeResult {
    let changed = state.guard_state.ack_upgrade_notice(&notice_id);
    if changed {
        guard_state_notifier(&app, &state).notify();
    }
    GuardChangeResult { changed }
}

#[tauri::command]
pub fn guard_set_budget_hard_stop<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    budget_id: String,
    hard_stop: bool,
) -> GuardChangeResult {
    let changed = state
        .guard_state
        .set_budget_hard_stop(&budget_id, hard_stop);
    if changed {
        guard_state_notifier(&app, &state).notify();
    }
    GuardChangeResult { changed }
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
    use crate::policy::{CommunityGuardStateProvider, DefaultNoopRegistry, GuardStateCore};
    use serde_json::json;
    use std::sync::Mutex;

    #[tokio::test]
    async fn upgrade_notice_commands_are_live_idempotent_and_community_noop() {
        use crate::policy::{ConvertedBudget, UpgradeNotice};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tauri::Listener;

        struct Provider {
            acknowledged: Mutex<bool>,
            hard_stop: Mutex<bool>,
            spent_cents: AtomicUsize,
            reads: AtomicUsize,
        }
        impl GuardStateProvider for Provider {
            fn snapshot(&self) -> GuardStateCore {
                self.reads.fetch_add(1, Ordering::SeqCst);
                CommunityGuardStateProvider::default().snapshot()
            }
            fn set_notifier(&self, _: GuardStateNotifier) {}
            fn upgrade_notice(&self) -> Option<UpgradeNotice> {
                if *self.acknowledged.lock().unwrap() {
                    return None;
                }
                let spent = self.spent_cents.load(Ordering::SeqCst);
                Some(UpgradeNotice {
                    notice_id: "conversion-1".into(),
                    converted_at: "2026-10-29T08:00:00Z".into(),
                    budgets: vec![ConvertedBudget {
                        budget_id: "provider:test".into(),
                        name: Some("API budget".into()),
                        provider_id: Some("test".into()),
                        daily_cap_usd: crate::usage::budget_migration::canonicalize_daily_budget(
                            "010.00",
                        )
                        .unwrap(),
                        spent_today_usd: rust_decimal::Decimal::new(spent as i64, 2)
                            .normalize()
                            .to_string(),
                        over_today: spent > 1000,
                    }],
                })
            }
            fn ack_upgrade_notice(&self, notice_id: &str) -> bool {
                let mut acknowledged = self.acknowledged.lock().unwrap();
                if notice_id != "conversion-1" || *acknowledged {
                    return false;
                }
                *acknowledged = true;
                true
            }
            fn set_budget_hard_stop(&self, budget_id: &str, hard_stop: bool) -> bool {
                let mut current = self.hard_stop.lock().unwrap();
                if budget_id != "provider:test" || *current == hard_stop {
                    return false;
                }
                *current = hard_stop;
                true
            }
        }

        for injected in [false, true] {
            let provider = Arc::new(Provider {
                acknowledged: Mutex::new(false),
                hard_stop: Mutex::new(true),
                spent_cents: AtomicUsize::new(975),
                reads: AtomicUsize::new(0),
            });
            let mut injections = crate::Injections::community();
            if injected {
                let provider = provider.clone();
                injections.guard_state = Box::new(move |_| Ok(provider));
            }
            let db = Arc::new(crate::Database::memory().unwrap());
            let state = AppState::new_with_injections(
                db.clone(),
                crate::secrets::unavailable_credential_store(),
                Arc::new(crate::quota::QuotaService::new(db)),
                injections,
            );
            let app = tauri::test::mock_app();
            app.manage(state);
            let events = Arc::new(Mutex::new(Vec::<Value>::new()));
            let sink = events.clone();
            app.listen(GUARD_STATE_CHANGED_EVENT, move |event| {
                sink.lock()
                    .unwrap()
                    .push(serde_json::from_str(event.payload()).unwrap());
            });
            let notice = guard_get_upgrade_notice(app.state());
            if injected {
                let notice = notice.unwrap();
                assert_eq!(notice.budgets[0].spent_today_usd, "9.75");
                assert!(!notice.budgets[0].over_today);
                chrono::DateTime::parse_from_rfc3339(&notice.converted_at).unwrap();
                provider.spent_cents.store(1234, Ordering::SeqCst);
                let live = guard_get_upgrade_notice(app.state()).unwrap();
                assert_eq!(
                    serde_json::to_value(&live).unwrap(),
                    json!({
                        "noticeId": "conversion-1", "convertedAt": "2026-10-29T08:00:00Z",
                        "budgets": [{"budgetId": "provider:test", "name": "API budget",
                            "providerId": "test", "dailyCapUsd": "10",
                            "spentTodayUsd": "12.34", "overToday": true}]
                    })
                );
                assert_eq!(
                    serde_json::from_value::<UpgradeNotice>(serde_json::to_value(&live).unwrap())
                        .unwrap(),
                    live
                );
                provider.spent_cents.store(1250, Ordering::SeqCst);
                assert_eq!(
                    guard_get_upgrade_notice(app.state()).unwrap().budgets[0].spent_today_usd,
                    "12.5"
                );
                provider.spent_cents.store(0, Ordering::SeqCst);
                assert_eq!(
                    guard_get_upgrade_notice(app.state()).unwrap().budgets[0].spent_today_usd,
                    "0"
                );
                let shared = ConvertedBudget {
                    budget_id: "shared".into(),
                    name: None,
                    provider_id: None,
                    daily_cap_usd: crate::usage::budget_migration::canonicalize_daily_budget(
                        "020.5000",
                    )
                    .unwrap(),
                    spent_today_usd: "0".into(),
                    over_today: false,
                };
                let wire = serde_json::to_value(&shared).unwrap();
                assert_eq!(
                    wire,
                    json!({"budgetId": "shared", "name": null,
                    "providerId": null, "dailyCapUsd": "20.5", "spentTodayUsd": "0",
                    "overToday": false})
                );
                assert_eq!(
                    serde_json::from_value::<ConvertedBudget>(wire).unwrap(),
                    shared
                );
            } else {
                assert!(notice.is_none());
            }
            assert!(events.lock().unwrap().is_empty());
            let reads = provider.reads.load(Ordering::SeqCst);
            for (id, expected) in [
                ("unknown", false),
                ("conversion-1", injected),
                ("conversion-1", false),
            ] {
                let result = guard_ack_upgrade_notice(app.handle().clone(), app.state(), id.into());
                assert_eq!(
                    serde_json::to_value(result).unwrap(),
                    json!({"changed": expected})
                );
            }
            assert!(guard_get_upgrade_notice(app.state()).is_none());
            // Each notifier invocation rebuilds the snapshot even if event dedup
            // suppresses emission. This catches duplicate calls hidden by dedup.
            assert_eq!(
                provider.reads.load(Ordering::SeqCst) - reads,
                usize::from(injected)
            );
            assert_eq!(events.lock().unwrap().len(), usize::from(injected));
            if injected {
                assert_eq!(
                    events.lock().unwrap()[0],
                    json!({"state": "ok", "blockedToday": 0,
                    "allowedToday": {"count": 0, "usd": "0"}, "pendingCount": 0, "pending": []})
                );
            }
            let reads = provider.reads.load(Ordering::SeqCst);
            for (id, enabled, expected) in [
                ("unknown", false, false),
                ("provider:test", false, injected),
                ("provider:test", false, false),
                ("provider:test", true, injected),
                ("provider:test", true, false),
            ] {
                let result = guard_set_budget_hard_stop(
                    app.handle().clone(),
                    app.state(),
                    id.into(),
                    enabled,
                );
                assert_eq!(result.changed, expected);
            }
            assert_eq!(
                provider.reads.load(Ordering::SeqCst) - reads,
                2 * usize::from(injected)
            );
            // The existing guard snapshot did not change, so no new event is invented.
            assert_eq!(events.lock().unwrap().len(), usize::from(injected));
        }
    }

    #[tokio::test]
    async fn injected_provider_and_registry_rebuild_the_same_command_and_event_view() {
        use crate::policy::testing::{meta, TestRegistry};
        use std::time::Duration;
        use tauri::Listener;
        struct Provider {
            core: Mutex<GuardStateCore>,
            notifier: Mutex<Option<GuardStateNotifier>>,
        }
        impl GuardStateProvider for Provider {
            fn snapshot(&self) -> GuardStateCore {
                self.core.lock().unwrap().clone()
            }
            fn set_notifier(&self, notifier: GuardStateNotifier) {
                *self.notifier.lock().unwrap() = Some(notifier);
            }
        }
        let provider = Arc::new(Provider {
            core: Mutex::new(GuardStateCore {
                state: GuardLevel::Capped,
                blocked_today: 7,
                allowed_today: AllowedToday {
                    count: 3,
                    usd: "12.50".into(),
                },
            }),
            notifier: Mutex::new(None),
        });
        let registry = Arc::new(TestRegistry::new(4));
        let mut injections = crate::Injections::community();
        let injected = provider.clone();
        injections.guard_state = Box::new(move |_| Ok(injected));
        let injected = registry.clone();
        injections.pending_registry = Box::new(move |_| Ok(injected));
        let db = Arc::new(crate::Database::memory().unwrap());
        let state = AppState::new_with_injections(
            db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(db)),
            injections,
        );
        let app = tauri::test::mock_app();
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = events.clone();
        app.listen(GUARD_STATE_CHANGED_EVENT, move |event| {
            sink.lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        let notifier = guard_state_notifier(app.handle(), &state);
        registry.set_notifier(notifier.clone());
        provider.set_notifier(notifier.clone());
        // Production wires before app.manage; early invalidations must work too.
        let ticket = registry
            .enqueue(meta("event", 1, "scope"), Duration::from_secs(60))
            .unwrap();
        app.manage(state);
        let capped = serde_json::to_value(get_guard_state(app.state()).await.unwrap()).unwrap();
        assert_eq!(capped["state"], "capped");
        assert_eq!(capped["blockedToday"], 7);
        assert_eq!(capped["allowedToday"], json!({"count": 3, "usd": "12.50"}));
        assert_eq!(capped["pendingCount"], 1);
        assert_eq!(capped["pending"][0]["pendingId"], "event");
        assert_eq!(events.lock().unwrap().as_slice(), &[capped.clone()]);
        notifier.notify();
        // Host budget publications must never overwrite injected enforcement.
        publish_guard_state_if_changed(app.handle(), UsageStatus::Yellow);
        assert_eq!(events.lock().unwrap().len(), 1);
        *provider.core.lock().unwrap() = GuardStateCore {
            state: GuardLevel::Killed,
            blocked_today: 11,
            allowed_today: AllowedToday {
                count: 5,
                usd: "20.75".into(),
            },
        };
        provider.notifier.lock().unwrap().clone().unwrap().notify();
        let killed = serde_json::to_value(get_guard_state(app.state()).await.unwrap()).unwrap();
        assert_eq!(killed["state"], "killed");
        assert_eq!(killed["blockedToday"], 11);
        assert_eq!(killed["allowedToday"], json!({"count": 5, "usd": "20.75"}));
        assert_eq!(killed["pending"], capped["pending"]);
        assert_eq!(events.lock().unwrap().last(), Some(&killed));
        notifier.notify();
        assert_eq!(events.lock().unwrap().len(), 2);
        drop(ticket);
        let empty = serde_json::to_value(get_guard_state(app.state()).await.unwrap()).unwrap();
        assert_eq!(empty["state"], "killed");
        assert_eq!(empty["blockedToday"], 11);
        assert_eq!(empty["allowedToday"], killed["allowedToday"]);
        assert_eq!(empty["pendingCount"], 0);
        assert_eq!(empty["pending"], json!([]));
        notifier.notify();
        let events = events.lock().unwrap();
        assert_eq!(events.as_slice(), &[capped, killed, empty]);
        registry.set_notifier(GuardStateNotifier::new(|| {}));
        provider.set_notifier(GuardStateNotifier::new(|| {}));
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
            assert_eq!(view.state, GuardLevel::Ok);
            assert_eq!(view.blocked_today, 0);
            assert_eq!(
                view.allowed_today,
                AllowedToday {
                    count: 0,
                    usd: "0".into()
                }
            );
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
            let provider = CommunityGuardStateProvider::default();
            provider.update_budget(status);
            let view = assemble_guard_state(&provider, &DefaultNoopRegistry);
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

    #[tokio::test]
    async fn guard_event_is_emitted_only_when_the_projected_view_changes() {
        use tauri::Listener;
        // Each app has its own deduplication state, unlike the old static cache.
        for _ in 0..2 {
            let app = tauri::test::mock_app();
            let mut state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
            let budget = Arc::new(Mutex::new(UsageStatus::Unknown));
            let source = budget.clone();
            state.tray_usage_service = Arc::new(
                crate::usage::tray_usage::TrayUsageService::new_with_dependencies(
                    Arc::new(move |_| {
                        let mut snapshot =
                            crate::usage::tray_snapshot::TrayUsageSnapshot::unknown(0);
                        snapshot.api_budget.status = *source.lock().unwrap();
                        Ok(snapshot)
                    }),
                    Arc::new(|| Box::pin(async { Ok(()) })),
                    Arc::new(chrono::Local::now),
                ),
            );
            app.manage(state);
            let events = Arc::new(Mutex::new(Vec::<Value>::new()));
            let sink = events.clone();
            app.listen(GUARD_STATE_CHANGED_EVENT, move |event| {
                sink.lock()
                    .unwrap()
                    .push(serde_json::from_str(event.payload()).unwrap());
            });
            for status in [
                UsageStatus::Green,
                UsageStatus::Unknown,
                UsageStatus::Yellow,
                UsageStatus::Red,
            ] {
                *budget.lock().unwrap() = status;
                let app_handle = app.handle().clone();
                app.state::<AppState>()
                    .tray_usage_service
                    .rebuild_from_persisted(move |snapshot| {
                        publish_guard_state_if_changed(&app_handle, snapshot.api_budget.status);
                    })
                    .await;
                let command =
                    serde_json::to_value(get_guard_state(app.state()).await.unwrap()).unwrap();
                assert_eq!(events.lock().unwrap().last(), Some(&command));
            }
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 2);
            for (event, level) in events.iter().zip(["ok", "warn"]) {
                assert_eq!(
                    *event,
                    json!({"state": level, "blockedToday": 0,
                    "allowedToday": {"count": 0, "usd": "0"}, "pendingCount": 0, "pending": []})
                );
            }
        }
    }
}
