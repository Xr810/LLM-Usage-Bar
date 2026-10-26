//! Durable, tier-independent retention prerequisite for activation.
use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::{error::AppError, store::Database};

const PENDING: &str = "retention_pending_target";

pub fn activation_retryable(stage: &'static str) -> AppError {
    AppError::coded_with_details(
        "LICENSE_ACTIVATION_RETRYABLE",
        "License activation could not persist retention. Please retry.",
        serde_json::json!({"stage": stage, "retryable": true}),
    )
}

pub struct RetentionStore {
    db: Arc<Database>,
}

impl RetentionStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn pending_target(&self) -> Option<u32> {
        match self.db.get_setting(PENDING) {
            Ok(value) => value.and_then(|s| s.parse().ok()),
            Err(error) => {
                log::error!("Cannot read pending retention: {error}");
                None
            }
        }
    }

    /// Activation calls this BEFORE publishing its new tier. Pending and
    /// high-water updates share one transaction. A failed high-water statement
    /// rolls back to a savepoint, retaining the pending record for startup.
    pub fn set_pending_target(&self, target: u32) -> Result<(), AppError> {
        self.db.write_pending_retention(Some(target))
    }

    /// Startup calls this even in community/Free builds. Never changes a tier.
    pub fn retry_pending(&self) -> Result<(), AppError> {
        self.db.retry_pending_retention()
    }
}

impl Database {
    pub(crate) fn retry_pending_retention(&self) -> Result<(), AppError> {
        self.write_pending_retention(None)
    }

    fn write_pending_retention(&self, target: Option<u32>) -> Result<(), AppError> {
        let stage = if target.is_some() {
            "pending_record"
        } else {
            "retention_high_water"
        };
        let result = (|| -> Result<Result<(), AppError>, AppError> {
            let mut conn = self.conn.lock()?;
            let tx = conn.transaction()?;
            if let Some(target) = target {
                tx.execute(
                    "INSERT INTO settings(key,value) VALUES (?1,?2)
                     ON CONFLICT(key) DO UPDATE SET value = MAX(CAST(value AS INTEGER), ?2)",
                    rusqlite::params![PENDING, target],
                )?;
            }
            let pending: Option<u32> = tx
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM settings WHERE key=?1",
                    [PENDING],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(pending) = pending else {
                return Ok(Ok(()));
            };
            tx.execute_batch("SAVEPOINT retention_raise")?;
            let raised = Database::raise_retention_high_water_on_conn(&tx, i64::from(pending));
            let outcome = match raised {
                Ok(_) => {
                    tx.execute("DELETE FROM settings WHERE key=?1", [PENDING])?;
                    Ok(())
                }
                Err(error) => {
                    log::error!("Cannot raise license retention high-water: {error}");
                    tx.execute_batch("ROLLBACK TO retention_raise")?;
                    Err(activation_retryable("retention_high_water"))
                }
            };
            tx.execute_batch("RELEASE retention_raise")?;
            tx.commit()?;
            Ok(outcome)
        })();
        result.unwrap_or_else(|error| {
            log::error!("Cannot persist license retention prerequisite: {error}");
            Err(activation_retryable(stage))
        })
    }
}
