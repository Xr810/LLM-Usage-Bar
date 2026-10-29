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
    pub budget_id: String,
    /// Shared budgets have no display name, matching BudgetAlert::scope_label.
    pub name: Option<String>,
    /// Existing reminder scope key: `shared` or `provider:<id>`.
    pub scope: String,
    pub daily_cap: String,
    /// Live at read time, not the spend recorded during conversion.
    pub spent_today: String,
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
    /// Notify after mutations, outside provider locks. Installation is inert.
    fn set_notifier(&self, notifier: GuardStateNotifier);
    /// Host's cached budget projection, used only by the community fallback.
    /// Injected enforcement providers keep their own authoritative state.
    fn update_budget(&self, _status: UsageStatus) {}
    /// Providers refresh spent_today and over_today on every read.
    fn upgrade_notice(&self) -> Option<UpgradeNotice> {
        None
    }
    /// Return true only for a newly acknowledged matching notice. Repeated or
    /// unknown IDs return false. The command owns notification for this mutation;
    /// implementations must not notify again from this hook.
    fn ack_upgrade_notice(&self, _notice_id: &str) -> bool {
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
