//! Public expiry and event orchestration; no token verification lives here.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use chrono::{DateTime, Duration, Utc};
use rusqlite::OptionalExtension;

use super::{LicenseBuild, LicenseProvider, LicenseState, LicenseStatus, LicenseTier};
use crate::{error::AppError, Database};

pub type LicensePublisher = Arc<dyn Fn(&LicenseStatus) -> Result<(), AppError> + Send + Sync>;

struct PendingPublication {
    provider: Arc<dyn LicenseProvider>,
    now: DateTime<Utc>,
    publisher: LicensePublisher,
}

#[derive(Default)]
struct Transitions {
    emitting: bool,
    queue: VecDeque<PendingPublication>,
}

type TransitionRegistry = Mutex<HashMap<usize, Weak<Mutex<Transitions>>>>;

// Wrappers around the same database handle must share transition ordering and
// emission ownership. Weak entries do not keep databases or publishers alive.
fn transitions_for(db: &Arc<Database>) -> Arc<Mutex<Transitions>> {
    static DISPATCHERS: OnceLock<TransitionRegistry> = OnceLock::new();
    let mut dispatchers = DISPATCHERS.get_or_init(Mutex::default).lock().unwrap();
    dispatchers.retain(|_, value| value.strong_count() != 0);
    let entry = dispatchers.entry(Arc::as_ptr(db) as usize).or_default();
    if let Some(transitions) = entry.upgrade() {
        return transitions;
    }
    let transitions = Arc::new(Mutex::new(Transitions::default()));
    *entry = Arc::downgrade(&transitions);
    transitions
}

pub struct LicenseLifecycle {
    provider: Arc<dyn LicenseProvider>,
    db: Arc<Database>,
    transitions: Arc<Mutex<Transitions>>,
    publisher: Mutex<Option<LicensePublisher>>,
}

impl LicenseLifecycle {
    pub fn new(provider: Arc<dyn LicenseProvider>, db: Arc<Database>) -> Self {
        Self {
            provider,
            transitions: transitions_for(&db),
            db,
            publisher: Mutex::new(None),
        }
    }

    pub fn set_publisher(&self, publisher: LicensePublisher) {
        *self.publisher.lock().unwrap() = Some(publisher);
    }

    fn status_at(provider: &dyn LicenseProvider, now: DateTime<Utc>) -> LicenseStatus {
        let mut status = provider.status();
        // Only the expiry and bounded grace duration determine this deadline.
        status.grace_until = None;
        if status.build == LicenseBuild::Community || status.tier == LicenseTier::Free {
            status.state = LicenseState::Free;
            return status;
        }
        status.state = LicenseState::Active;
        if let Some(expiry) = status
            .expires_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            let grace = expiry + Duration::days(i64::from(status.grace_days.unwrap_or(14).min(14)));
            status.grace_until = Some(grace.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            if now >= grace {
                status.state = LicenseState::Free;
                status.tier = LicenseTier::Free;
            } else if now >= expiry {
                status.state = LicenseState::Grace;
            }
        }
        status
    }

    pub fn reconcile(&self, now: DateTime<Utc>) -> Result<(), AppError> {
        {
            let mut transitions = self.transitions.lock()?;
            self.enqueue(&mut transitions, now)?;
        }
        self.drain()
    }

    fn enqueue(&self, transitions: &mut Transitions, now: DateTime<Utc>) -> Result<(), AppError> {
        if let Some(publisher) = self.publisher.lock()?.clone() {
            transitions.queue.push_back(PendingPublication {
                provider: self.provider.clone(),
                now,
                publisher,
            });
        }
        Ok(())
    }

    fn drain(&self) -> Result<(), AppError> {
        {
            let mut transitions = self.transitions.lock()?;
            if transitions.emitting {
                // Reentrant/concurrent commands enqueue and return. The current
                // emitter drains them after its callback, never recursively.
                return Ok(());
            }
            transitions.emitting = true;
        }
        let mut failure = None;
        loop {
            let next = {
                let mut transitions = self.transitions.lock()?;
                match transitions.queue.pop_front() {
                    Some(next) => next,
                    None => {
                        transitions.emitting = false;
                        break;
                    }
                }
            };
            // Re-read the originating provider at delivery, not enqueue time:
            // a prior callback may have activated or deactivated it meanwhile.
            let status = Self::status_at(next.provider.as_ref(), next.now);
            if status.build == LicenseBuild::Community {
                continue;
            }
            if let Err(error) = self.publish(&status, &next.publisher) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn publish(
        &self,
        status: &LicenseStatus,
        publisher: &LicensePublisher,
    ) -> Result<(), AppError> {
        let key = serde_json::to_string(&(
            status.tier,
            status.state,
            &status.expires_at,
            &status.grace_until,
        ))
        .map_err(|_| AppError::internal("Cannot serialize license state."))?;
        let previous = self.db.get_setting("license_last_emitted_state")?;
        if previous.as_deref() == Some(&key) {
            return Ok(());
        }
        // No DB mutex, transaction, transition or publisher lock crosses the
        // callback. It may synchronously read the DB or change the license.
        publisher(status)?;
        let mut conn = self.db.conn.lock()?;
        let tx = conn.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='license_last_emitted_state'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current != previous {
            return Err(AppError::internal(
                "License acknowledgement changed during delivery; retry required.",
            ));
        }
        tx.execute(
            "INSERT INTO settings(key,value) VALUES ('license_last_emitted_state',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value IS ?2",
            rusqlite::params![key, previous],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn drain_or_log(&self) {
        if let Err(error) = self.drain() {
            log::error!("License event delivery will retry: {error}");
        }
    }
}

impl LicenseProvider for LicenseLifecycle {
    fn status(&self) -> LicenseStatus {
        let _guard = self.transitions.lock().unwrap();
        Self::status_at(self.provider.as_ref(), Utc::now())
    }

    fn activate(&self, token: &str) -> Result<LicenseStatus, AppError> {
        super::license::validate_license_token(token)?;
        let status = {
            let mut transitions = self.transitions.lock()?;
            self.provider.activate(token)?;
            let now = Utc::now();
            let status = Self::status_at(self.provider.as_ref(), now);
            self.enqueue(&mut transitions, now)?;
            status
        };
        self.drain_or_log();
        Ok(status)
    }

    fn deactivate(&self) -> Result<(), AppError> {
        {
            let mut transitions = self.transitions.lock()?;
            self.provider.deactivate()?;
            self.enqueue(&mut transitions, Utc::now())?;
        }
        self.drain_or_log();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
