//! Public expiry and event orchestration; no token verification lives here.
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use rusqlite::OptionalExtension;

use super::{LicenseBuild, LicenseProvider, LicenseState, LicenseStatus, LicenseTier};
use crate::{error::AppError, Database};

pub type LicensePublisher = Arc<dyn Fn(&LicenseStatus) -> Result<(), AppError> + Send + Sync>;

pub struct LicenseLifecycle {
    provider: Arc<dyn LicenseProvider>,
    db: Arc<Database>,
    transition: Mutex<()>,
    publisher: Mutex<Option<LicensePublisher>>,
}

impl LicenseLifecycle {
    pub fn new(provider: Arc<dyn LicenseProvider>, db: Arc<Database>) -> Self {
        Self {
            provider,
            db,
            transition: Mutex::new(()),
            publisher: Mutex::new(None),
        }
    }

    pub fn set_publisher(&self, publisher: LicensePublisher) {
        *self.publisher.lock().unwrap() = Some(publisher);
    }

    fn status_at(&self, now: DateTime<Utc>) -> LicenseStatus {
        let mut status = self.provider.status();
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
            let grace = expiry + Duration::days(14);
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
        let _guard = self.transition.lock()?;
        self.publish(&self.status_at(now))
    }

    fn publish(&self, status: &LicenseStatus) -> Result<(), AppError> {
        if status.build == LicenseBuild::Community {
            return Ok(());
        }
        let publisher = self.publisher.lock()?.clone();
        let Some(publisher) = publisher else {
            return Ok(());
        };
        let key = serde_json::to_string(&(
            status.tier,
            status.state,
            &status.expires_at,
            &status.grace_until,
        ))
        .map_err(|_| AppError::internal("Cannot serialize license state."))?;
        // The DB mutex and transaction serialize the compare-and-set across
        // lifecycle instances too. Do not acknowledge until emit succeeds.
        let mut conn = self.db.conn.lock()?;
        let tx = conn.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='license_last_emitted_state'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if previous.as_deref() == Some(&key) {
            return Ok(());
        }
        let won = tx.execute(
            "INSERT INTO settings(key,value) VALUES ('license_last_emitted_state',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value=?2",
            rusqlite::params![key, previous],
        )?;
        if won == 1 {
            publisher(status)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn publish_or_log(&self, status: &LicenseStatus) {
        if let Err(error) = self.publish(status) {
            log::error!("License event delivery will retry: {error}");
        }
    }
}

impl LicenseProvider for LicenseLifecycle {
    fn status(&self) -> LicenseStatus {
        let _guard = self.transition.lock().unwrap();
        self.status_at(Utc::now())
    }

    fn activate(&self, token: &str) -> Result<LicenseStatus, AppError> {
        super::license::validate_license_token(token)?;
        let _guard = self.transition.lock()?;
        self.provider.activate(token)?;
        let status = self.status_at(Utc::now());
        self.publish_or_log(&status);
        Ok(status)
    }

    fn deactivate(&self) -> Result<(), AppError> {
        let _guard = self.transition.lock()?;
        self.provider.deactivate()?;
        self.publish_or_log(&self.status_at(Utc::now()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
