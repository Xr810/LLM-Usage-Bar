//! Deterministic test registry only. Not compiled into any product build.
use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};
use tokio::time::Instant;

struct Entry {
    meta: PendingMeta,
    deadline: Instant,
    sender: oneshot::Sender<HoldOutcome>,
}
type Publisher = Arc<dyn Fn(Vec<PendingMeta>) + Send + Sync>;
#[derive(Default)]
struct Inner {
    queue: Vec<Entry>,
    allowance: u32,
    active: usize,
    publisher: Option<Publisher>,
}

#[derive(Clone)]
pub(crate) struct TestRegistry {
    inner: Arc<Mutex<Inner>>,
    changed: Arc<Notify>,
    max: usize,
}

impl TestRegistry {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            changed: Arc::new(Notify::new()),
            max,
        }
    }
    fn publish(&self) {
        let publisher = self.inner.lock().unwrap().publisher.clone();
        if let Some(publisher) = publisher {
            publisher(self.view());
        }
        self.changed.notify_waiters();
    }
    pub async fn wait_count(&self, count: usize) {
        loop {
            let changed = self.changed.notified();
            if self.view().len() == count {
                return;
            }
            changed.await;
        }
    }
    pub fn finish(&self, id: &str, outcome: HoldOutcome) -> bool {
        let entry = {
            let mut inner = self.inner.lock().unwrap();
            inner
                .queue
                .iter()
                .position(|e| e.meta.pending_id == id)
                .map(|i| inner.queue.remove(i))
        };
        if let Some(entry) = entry {
            let outcome = if Instant::now() >= entry.deadline {
                HoldOutcome::Timeout
            } else {
                outcome
            };
            let released = matches!(outcome, HoldOutcome::Released(_));
            let sent = entry.sender.send(outcome).is_ok();
            self.publish();
            return sent && released;
        }
        false
    }
}

impl PendingRegistry for TestRegistry {
    fn enqueue(&self, meta: PendingMeta, timeout: Duration) -> Result<PendingTicket, EnqueueError> {
        let (sender, receiver) = oneshot::channel();
        let id = meta.pending_id.clone();
        let deadline = Instant::now() + timeout;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.queue.len() >= self.max {
                return Err(EnqueueError::Full);
            }
            inner.queue.push(Entry {
                meta,
                deadline,
                sender,
            });
            inner.queue.sort_by_key(|entry| entry.meta.held_at);
            inner.active += 1;
        }
        self.publish();
        let timer = self.clone();
        let timeout_id = id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            timer.finish(&timeout_id, HoldOutcome::Timeout);
        });
        let registry = self.clone();
        Ok(PendingTicket::new(receiver, move || {
            task.abort();
            {
                let mut inner = registry.inner.lock().unwrap();
                inner.queue.retain(|entry| entry.meta.pending_id != id);
                inner.active -= 1;
            }
            registry.publish();
        }))
    }
    fn release(&self, pending_id: Option<&str>) -> ReleaseResult {
        let id = pending_id
            .map(str::to_string)
            .or_else(|| self.view().first().map(|m| m.pending_id.clone()));
        let released = id
            .as_deref()
            .is_some_and(|id| self.finish(id, HoldOutcome::Released(ReleasedBy::Once)));
        ReleaseResult {
            released,
            pending_id: if released { id } else { None },
        }
    }
    fn allow_more_today(&self, amount_usd: Option<&str>) -> AllowanceResult {
        // Deliberately deterministic test policy: the head's opaque scope wins.
        let view = self.view();
        let mut released_count = 0;
        if let Some(head) = view.first() {
            for entry in &view {
                if entry.scope == head.scope
                    && self.finish(
                        &entry.pending_id,
                        HoldOutcome::Released(ReleasedBy::MoreToday),
                    )
                {
                    released_count += 1;
                }
            }
        }
        let mut inner = self.inner.lock().unwrap();
        inner.allowance += amount_usd.unwrap_or("5").parse::<u32>().unwrap();
        AllowanceResult {
            released_count,
            allowance_usd: inner.allowance.to_string(),
        }
    }
    fn deny(&self, pending_id: Option<&str>) -> DenyResult {
        let mut denied_count = 0;
        for entry in self.view() {
            if pending_id.is_none_or(|id| id == entry.pending_id) {
                self.finish(&entry.pending_id, HoldOutcome::Denied);
                denied_count += 1;
            }
        }
        DenyResult { denied_count }
    }
    fn drain(&self) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            for entry in self.view() {
                self.finish(&entry.pending_id, HoldOutcome::Shutdown);
            }
            loop {
                let changed = self.changed.notified();
                if self.inner.lock().unwrap().active == 0 {
                    return;
                }
                changed.await;
            }
        })
    }
    fn set_publisher(&self, publisher: Publisher) {
        self.inner.lock().unwrap().publisher = Some(publisher);
    }
    fn view(&self) -> Vec<PendingMeta> {
        self.inner
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|e| e.meta.clone())
            .collect()
    }
}

pub(crate) fn meta(id: &str, held_at: i64, rule: &str) -> PendingMeta {
    PendingMeta {
        pending_id: id.into(),
        request_id: format!("request-{id}"),
        kind: HoldKind::HardCap,
        scope: HoldScope::DailyCap {
            rule_id: rule.into(),
        },
        provider: None,
        client: "claude".into(),
        session_id: None,
        held_at,
        expires_at: held_at + 15_000,
        reason: "test hold".into(),
    }
}

#[tokio::test(start_paused = true)]
async fn registry_contract_fifo_scope_allowance_deny_timeout_and_drop() {
    let registry = TestRegistry::new(4);
    let mut later = registry
        .enqueue(meta("later", 20, "A"), Duration::from_secs(15))
        .unwrap();
    let mut first = registry
        .enqueue(meta("first", 10, "B"), Duration::from_secs(15))
        .unwrap();
    let mut same = registry
        .enqueue(meta("same", 30, "A"), Duration::from_secs(15))
        .unwrap();
    let mut other = registry
        .enqueue(meta("other", 40, "B"), Duration::from_secs(15))
        .unwrap();
    assert!(matches!(
        registry.enqueue(meta("overflow", 40, "A"), Duration::from_secs(15)),
        Err(EnqueueError::Full)
    ));
    assert_eq!(registry.release(None).pending_id.as_deref(), Some("first"));
    assert_eq!(
        (&mut first.receiver).await.unwrap(),
        HoldOutcome::Released(ReleasedBy::Once)
    );
    assert!(!registry.release(Some("first")).released);
    assert_eq!(
        registry.allow_more_today(Some("2")),
        AllowanceResult {
            released_count: 2,
            allowance_usd: "2".into()
        }
    );
    for ticket in [&mut later, &mut same] {
        assert_eq!(
            (&mut ticket.receiver).await.unwrap(),
            HoldOutcome::Released(ReleasedBy::MoreToday)
        );
    }
    assert_eq!(
        registry
            .view()
            .iter()
            .map(|m| m.pending_id.as_str())
            .collect::<Vec<_>>(),
        vec!["other"]
    );
    assert!(registry.release(Some("other")).released);
    assert_eq!(
        (&mut other.receiver).await.unwrap(),
        HoldOutcome::Released(ReleasedBy::Once)
    );
    assert_eq!(registry.allow_more_today(None).allowance_usd, "7");
    let mut expired = registry
        .enqueue(meta("expired", 1, "A"), Duration::from_secs(15))
        .unwrap();
    tokio::time::advance(Duration::from_secs(15)).await;
    assert!(!registry.release(Some("expired")).released);
    assert_eq!((&mut expired.receiver).await.unwrap(), HoldOutcome::Timeout);
    let mut denied = registry
        .enqueue(meta("deny", 1, "A"), Duration::from_secs(15))
        .unwrap();
    assert_eq!(registry.deny(Some("deny")).denied_count, 1);
    assert_eq!((&mut denied.receiver).await.unwrap(), HoldOutcome::Denied);
    assert_eq!(registry.deny(None).denied_count, 0);
    let ticket = registry
        .enqueue(meta("drop", 1, "A"), Duration::from_secs(15))
        .unwrap();
    drop(ticket);
    assert!(registry.view().is_empty());
    assert!(!registry.release(Some("drop")).released);
}

#[test]
fn noop_and_wire_contract() {
    let registry = DefaultNoopRegistry;
    let spec = HoldSpec {
        kind: HoldKind::IdentitySwitch,
        scope: HoldScope::Identity {
            project_key: "p".into(),
        },
        timeout: Duration::from_secs(60),
    };
    let wire = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({"kind":"identity_switch","scope":{"kind":"identity","projectKey":"p"},"timeout":{"secs":60,"nanos":0}})
    );
    assert_eq!(serde_json::from_value::<HoldSpec>(wire).unwrap(), spec);
    assert_eq!(
        serde_json::to_value(HoldOutcome::Released(ReleasedBy::MoreToday)).unwrap(),
        serde_json::json!({"kind":"released","releasedBy":"more_today"})
    );
    assert_eq!(
        serde_json::to_value(HoldOutcome::Timeout).unwrap(),
        serde_json::json!({"kind":"timeout"})
    );
    assert_eq!(
        registry.release(None),
        ReleaseResult {
            released: false,
            pending_id: None
        }
    );
    assert_eq!(registry.deny(None).denied_count, 0);
    assert_eq!(
        registry.allow_more_today(None),
        AllowanceResult {
            released_count: 0,
            allowance_usd: "0".into()
        }
    );
    assert!(registry.view().is_empty());
    let value = serde_json::to_value(meta("one", 123, "opaque")).unwrap();
    assert_eq!(
        value["scope"],
        serde_json::json!({"kind":"daily_cap", "ruleId":"opaque"})
    );
    assert_eq!(value["pendingId"], "one");
    assert_eq!(value["heldAt"], 123);
    assert_eq!(
        serde_json::to_value(HoldScope::Identity {
            project_key: "project".into()
        })
        .unwrap(),
        serde_json::json!({"kind":"identity", "projectKey":"project"})
    );
}
