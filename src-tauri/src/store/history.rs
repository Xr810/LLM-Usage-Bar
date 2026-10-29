//! Read-only history visibility. Retention, ingestion, backup and sync are separate.
use super::Database;
use crate::error::AppError;
use chrono::{Local, NaiveDate, TimeZone};
use std::sync::Arc;

/// Build-owned inclusive local-date floor. None leaves history unrestricted.
/// Implementations must not apply retention or mutate historical rows.
pub trait HistoryVisibilityPolicy: Send + Sync {
    fn cutoff(&self) -> Option<NaiveDate>;
}

pub struct CommunityHistoryVisibilityPolicy;

impl HistoryVisibilityPolicy for CommunityHistoryVisibilityPolicy {
    fn cutoff(&self) -> Option<NaiveDate> {
        None
    }
}

/// Capture once per logical read, including its count and aggregate queries.
/// Billing-history exporters must use the same snapshot for all exported rows.
#[derive(Debug, Clone, Copy)]
pub struct HistoryVisibility {
    pub cutoff: Option<NaiveDate>,
}

impl HistoryVisibility {
    pub fn clamp_start(&self, start: Option<i64>) -> Result<Option<i64>, AppError> {
        let Some(day) = self.cutoff else {
            return Ok(start);
        };
        let midnight = day
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Local.from_local_datetime(&value).earliest())
            .ok_or_else(|| AppError::internal("Could not determine history cutoff."))?
            .timestamp();
        Ok(Some(start.map_or(midnight, |start| start.max(midnight))))
    }
}

impl Database {
    pub(crate) fn install_history_visibility(&self, policy: &Arc<dyn HistoryVisibilityPolicy>) {
        *self
            .history_visibility
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(Arc::downgrade(policy));
    }

    /// Shared history-read boundary; never call while holding the DB connection.
    /// Providers may consult the same authoritative database to resolve policy.
    pub fn history_visibility(&self) -> HistoryVisibility {
        let provider = self
            .history_visibility
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        HistoryVisibility {
            cutoff: provider.and_then(|provider| provider.cutoff()),
        }
    }
}
