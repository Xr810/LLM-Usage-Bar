use crate::error::AppError;
use crate::services::ClaudeCliAuthStatus;
use crate::store::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_claude_cli_auth_status(
    state: State<'_, AppState>,
) -> Result<ClaudeCliAuthStatus, AppError> {
    let mut status = state.claude_cli_auth_service.status().await;
    // The CLI reports authentication; quota arrives separately as local samples.
    // Composing them here keeps the auth service off the database while still
    // letting the card say when the quota it shows was last observed.
    status.last_quota_sample_at = state
        .db
        .latest_quota_snapshot(crate::usage::system_providers::CLAUDE_SUBSCRIPTION_ID)?
        .map(|snapshot| snapshot.fetched_at);
    Ok(status)
}

#[tauri::command]
pub fn start_claude_cli_login(state: State<'_, AppState>) -> Result<(), AppError> {
    state.claude_cli_auth_service.start_login()
}

#[tauri::command]
pub async fn logout_claude_cli(
    state: State<'_, AppState>,
) -> Result<ClaudeCliAuthStatus, AppError> {
    Ok(state.claude_cli_auth_service.logout().await)
}
