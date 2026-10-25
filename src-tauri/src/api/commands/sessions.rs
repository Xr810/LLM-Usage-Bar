//! BE-22: read-only `list_sessions` (contract C14).
//!
//! Community builds have no session caps, so `state` is `ok` and `capUsd` is
//! omitted. This command does not migrate the database.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::usage::session_list::{list_sessions_at, SessionListPage};
use tauri::State;

#[tauri::command]
pub fn list_sessions(
    active_only: Option<bool>,
    limit: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<SessionListPage, AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| AppError::internal("Could not list sessions."))?;
    list_sessions_at(
        &state.db,
        now,
        active_only.unwrap_or(false),
        limit,
        cursor.as_deref(),
    )
}
