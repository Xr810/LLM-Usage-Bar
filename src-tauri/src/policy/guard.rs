//! Public guard snapshot contract. Enforcement remains build-owned.
use crate::model::UsageStatus;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

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
pub struct GuardStateCore {
    pub state: GuardLevel,
    pub blocked_today: u32,
    pub allowed_today: AllowedToday,
}

/// Converted reminder metadata; amounts reuse the budget model's decimal USD strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertedBudget {
    /// `shared` or `provider:<id>`, matching the reminder scope key.
    pub budget_id: String,
    /// Shared budgets have no display name, matching BudgetAlert::scope_label.
    pub name: Option<String>,
    /// None for the shared budget.
    pub provider_id: Option<String>,
    /// Canonical decimal USD, using canonicalize_daily_budget for settings caps.
    pub daily_cap_usd: String,
    /// Live at read time, not the spend recorded during conversion.
    /// Decimal::normalize().to_string(), as in existing budget spending views.
    pub spent_today_usd: String,
    /// Provider-owned live comparison with the daily cap.
    pub over_today: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeNotice {
    pub notice_id: String,
    /// RFC3339 conversion timestamp, supplied by the provider.
    pub converted_at: String,
    pub budgets: Vec<ConvertedBudget>,
}

/// Invalidation only: the host reads both authoritative snapshots on every call.
#[derive(Clone)]
pub struct GuardStateNotifier(Arc<dyn Fn() + Send + Sync>);

impl GuardStateNotifier {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(notify))
    }
    pub fn notify(&self) {
        (self.0)();
    }
}

pub trait GuardStateProvider: Send + Sync {
    fn snapshot(&self) -> GuardStateCore;
    /// Returns whether state changed. The command invalidates the shared view
    /// after this returns, outside provider locks. Community is inert.
    /// Injected providers clear a true kill automatically at the next local
    /// midnight; false clears it early. Persistence and scheduling are provider-owned.
    fn set_killed(&self, _killed: bool) -> bool {
        false
    }
    /// Notify after mutations, outside provider locks. Installation is inert.
    fn set_notifier(&self, notifier: GuardStateNotifier);
    /// Host's cached budget projection, used only by the community fallback.
    /// Injected enforcement providers keep their own authoritative state.
    fn update_budget(&self, _status: UsageStatus) {}
    /// Private providers convert only daily budgets actually in effect under
    /// the current api_budget_mode. Monthly budgets remain warn-only and are
    /// not converted (pending owner confirmation). Conversion/storage is private.
    /// Providers refresh spent_today_usd and over_today on every read, using
    /// the existing budget USD normalizers; the public host only returns shape.
    fn upgrade_notice(&self) -> Option<UpgradeNotice> {
        None
    }
    /// Return true only for a newly acknowledged matching notice. Repeated or
    /// unknown IDs return false. The command owns notification for this mutation;
    /// implementations must not notify again from this hook.
    fn ack_upgrade_notice(&self, _notice_id: &str) -> bool {
        false
    }
    /// On Pro, edit ANY daily budget currently in effect under the active
    /// api_budget_mode (`shared` or `provider:<id>`), not only converted budgets.
    /// Also used by the settings page hard-stop toggle. Storage and enforcement
    /// are private; community remains warn-only. Return true only
    /// on change, false for unknown IDs or repeats. The command owns notification.
    fn set_budget_hard_stop(&self, _budget_id: &str, _hard_stop: bool) -> bool {
        false
    }
}

pub struct CommunityGuardStateProvider(Mutex<UsageStatus>);

impl Default for CommunityGuardStateProvider {
    fn default() -> Self {
        Self(Mutex::new(UsageStatus::Unknown))
    }
}

impl GuardStateProvider for CommunityGuardStateProvider {
    fn snapshot(&self) -> GuardStateCore {
        let state = match *self.0.lock().unwrap_or_else(|p| p.into_inner()) {
            UsageStatus::Yellow | UsageStatus::Red => GuardLevel::Warn,
            UsageStatus::Green | UsageStatus::Unknown => GuardLevel::Ok,
        };
        GuardStateCore {
            state,
            blocked_today: 0,
            allowed_today: AllowedToday {
                count: 0,
                usd: "0".into(),
            },
        }
    }
    fn set_notifier(&self, _: GuardStateNotifier) {}
    fn update_budget(&self, status: UsageStatus) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = status;
    }
}
