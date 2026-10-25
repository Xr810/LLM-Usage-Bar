//! BE-17 command adapters. Filesystem policy lives in `route::takeover`.
//!
//! On Windows these commands do not read Credential Manager and do not write
//! config. A Windows write returns `FEATURE_UNAVAILABLE` with
//! `{feature: "takeover_write", reason: "platform"}`. Cursor stays
//! `UNSUPPORTED_TOOL`.
//!
//! `set_claude_subscription_proxy(false)` is the only path that disables the
//! Claude subscription proxy.

use crate::app_state::AppState;
use crate::config::get_home_dir;
use crate::error::AppError;
use crate::route::takeover::{
    apply_takeover_at, detect_claude_auth_at, detect_tools_at, macos_bin_dirs,
    set_claude_subscription_proxy_at, ApplyTakeoverResult, ClaudeAuthView, DetectEnv,
    TakeoverLayout, ToolDetectRow,
};
#[cfg(target_os = "windows")]
use crate::route::takeover::{
    windows_subscription_proxy_unavailable, windows_takeover_write_error, windows_tool_stub,
};
use tauri::State;

fn host_env() -> DetectEnv {
    DetectEnv {
        anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        anthropic_auth_token: std::env::var("ANTHROPIC_AUTH_TOKEN").ok(),
        openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
    }
}

fn host_layout(state: &AppState) -> TakeoverLayout {
    let home = get_home_dir();
    TakeoverLayout {
        bin_dirs: macos_bin_dirs(&home),
        router_port: crate::api::router::read_router_port(&state.db),
        env: host_env(),
        home,
    }
}

#[tauri::command]
pub fn detect_claude_auth_mode() -> Result<ClaudeAuthView, AppError> {
    #[cfg(target_os = "windows")]
    {
        // TODO(BE-17): Windows auth detection is not implemented.
        return Ok(ClaudeAuthView {
            mode: crate::route::takeover::AuthMode::Unknown,
            evidence: vec!["windows_detection_pending".to_string()],
        });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = get_home_dir();
        Ok(detect_claude_auth_at(&TakeoverLayout {
            bin_dirs: Vec::new(),
            router_port: crate::api::router::DEFAULT_ROUTER_PORT,
            env: host_env(),
            home,
        }))
    }
}

#[tauri::command]
pub fn set_claude_subscription_proxy(
    enabled: bool,
    terms_ack_at: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        let _ = (enabled, terms_ack_at, state);
        // TODO(BE-17): Windows config writes are not implemented.
        return Err(windows_subscription_proxy_unavailable());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .map_err(|_| AppError::internal("Could not read the clock."))?;
        set_claude_subscription_proxy_at(
            &host_layout(&state),
            &state.db,
            enabled,
            terms_ack_at,
            now,
        )
    }
}

#[tauri::command]
pub fn detect_tools(state: State<'_, AppState>) -> Result<Vec<ToolDetectRow>, AppError> {
    #[cfg(target_os = "windows")]
    {
        let _ = state;
        return Ok(windows_tool_stub());
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(detect_tools_at(&host_layout(&state)))
    }
}

#[tauri::command]
pub fn apply_takeover(
    tool: String,
    enable: bool,
    state: State<'_, AppState>,
) -> Result<ApplyTakeoverResult, AppError> {
    #[cfg(target_os = "windows")]
    {
        let _ = (enable, state);
        // TODO(BE-17): Windows config writes are not implemented.
        return Err(windows_takeover_write_error(&tool));
    }
    #[cfg(not(target_os = "windows"))]
    {
        apply_takeover_at(&host_layout(&state), &tool, enable)
    }
}
