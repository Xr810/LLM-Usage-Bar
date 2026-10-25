//! BE-5b: `get_quota_status` (契约 C4).
//!
//! The command only reads stored snapshots. A provider whose last read failed
//! is still a successful response with `state` of `stale` or `error`.

use crate::app_state::AppState;
use crate::error::AppError;
use crate::quota::source_status::{
    quota_status_at, quota_updated_payload, ProviderQuotaStatus, QUOTA_UPDATED_EVENT,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn get_quota_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderQuotaStatus>, AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| AppError::internal("Could not read quota status."))?;
    let rows = quota_status_at(&state.db, now)
        .map_err(|_| AppError::internal("Could not read quota status."))?;
    let _ = app.emit(QUOTA_UPDATED_EVENT, quota_updated_payload(&rows));
    Ok(rows)
}
