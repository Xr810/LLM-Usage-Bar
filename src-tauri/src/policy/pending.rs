//! Public Hold transport contract. Queue ownership and policy remain injected.
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldKind {
    HardCap,
    IdentitySwitch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoldScope {
    DailyCap {
        #[serde(rename = "ruleId")]
        rule_id: String,
    },
    Identity {
        #[serde(rename = "projectKey")]
        project_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldSpec {
    pub kind: HoldKind,
    pub scope: HoldScope,
    pub timeout: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMeta {
    pub pending_id: String,
    pub request_id: String,
    pub kind: HoldKind,
    pub scope: HoldScope,
    pub provider: Option<String>,
    pub client: String,
    pub session_id: Option<String>,
    /// Unix milliseconds.
    pub held_at: i64,
    pub expires_at: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleasedBy {
    Once,
    MoreToday,
    AutoIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOutcome {
    Released(ReleasedBy),
    Denied,
    Timeout,
    Shutdown,
}

/// Dropping the waiter removes its queue entry, including task cancellation.
pub struct PendingTicket {
    pub receiver: oneshot::Receiver<HoldOutcome>,
    dequeue: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl PendingTicket {
    pub fn new(
        receiver: oneshot::Receiver<HoldOutcome>,
        dequeue: impl FnOnce() + Send + Sync + 'static,
    ) -> Self {
        Self {
            receiver,
            dequeue: Some(Box::new(dequeue)),
        }
    }
}

impl Drop for PendingTicket {
    fn drop(&mut self) {
        if let Some(dequeue) = self.dequeue.take() {
            dequeue();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueError {
    Unavailable,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseResult {
    pub released: bool,
    pub pending_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowanceResult {
    pub released_count: usize,
    pub allowance_usd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DenyResult {
    pub denied_count: usize,
}

/// Commands delegate opaque scope selection to this registry. Implementations
/// serialize terminal outcomes, publish changed views on enqueue/dequeue, and
/// own timeout scheduling. No private queue or allowance logic lives here.
pub trait PendingRegistry: Send + Sync {
    fn enqueue(
        &self,
        meta: PendingMeta,
        timeout: std::time::Duration,
    ) -> Result<PendingTicket, EnqueueError>;
    /// None selects the earliest heldAt entry. Gone IDs are idempotent false.
    fn release(&self, pending_id: Option<&str>) -> ReleaseResult;
    /// None uses the configured default. Scope selection, local-day allowance
    /// reset and same-scope FIFO release belong exclusively to the registry.
    fn allow_more_today(&self, amount_usd: Option<&str>) -> AllowanceResult;
    /// None denies all entries.
    fn deny(&self, pending_id: Option<&str>) -> DenyResult;
    /// Resolve all waiters with Shutdown; must not block.
    fn drain(&self);
    fn view(&self) -> Vec<PendingMeta>;
}

#[derive(Default)]
pub struct DefaultNoopRegistry;

impl PendingRegistry for DefaultNoopRegistry {
    fn enqueue(
        &self,
        _: PendingMeta,
        _: std::time::Duration,
    ) -> Result<PendingTicket, EnqueueError> {
        Err(EnqueueError::Unavailable)
    }
    fn release(&self, _: Option<&str>) -> ReleaseResult {
        ReleaseResult {
            released: false,
            pending_id: None,
        }
    }
    fn allow_more_today(&self, _: Option<&str>) -> AllowanceResult {
        AllowanceResult {
            released_count: 0,
            allowance_usd: "0".into(),
        }
    }
    fn deny(&self, _: Option<&str>) -> DenyResult {
        DenyResult { denied_count: 0 }
    }
    fn drain(&self) {}
    fn view(&self) -> Vec<PendingMeta> {
        Vec::new()
    }
}
