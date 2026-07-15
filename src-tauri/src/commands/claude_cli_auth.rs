use crate::error::AppError;
use crate::services::ClaudeCliAuthStatus;
use crate::store::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_claude_cli_auth_status(
    state: State<'_, AppState>,
) -> Result<ClaudeCliAuthStatus, AppError> {
    Ok(state.claude_cli_auth_service.status().await)
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
