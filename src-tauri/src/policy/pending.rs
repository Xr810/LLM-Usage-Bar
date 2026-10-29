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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldSpec {
    pub kind: HoldKind,
    pub scope: HoldScope,
    #[serde(rename = "timeoutSecs", with = "timeout_secs")]
    pub timeout: std::time::Duration,
}

mod timeout_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(timeout: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(timeout.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_secs)
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "releasedBy", rename_all = "snake_case")]
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
    /// None uses the configured default. Add allowance until local midnight
    /// without changing the cap, then release all current eligible daily-cap
    /// and identity-switch entries FIFO, never killed state. Provenance remains
    /// MoreToday (wire: more_today) for both kinds. The injected registry owns
    /// identity-baseline updates; later new key switches still require admission.
    fn allow_more_today(&self, amount_usd: Option<&str>) -> AllowanceResult;
    /// None denies all entries.
    fn deny(&self, pending_id: Option<&str>) -> DenyResult;
    /// Resolve all waiters with Shutdown, then await their dequeue acknowledgments.
    /// The host bounds this future to two seconds on exit.
    fn drain(&self) -> futures::future::BoxFuture<'_, ()>;
    /// Notify after each queue mutation, outside queue locks. The host rebuilds
    /// the whole view and filters duplicates. Installation must be inert.
    fn set_notifier(&self, notifier: super::GuardStateNotifier);
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
    fn drain(&self) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn set_notifier(&self, _: super::GuardStateNotifier) {}
    fn view(&self) -> Vec<PendingMeta> {
        Vec::new()
    }
}
