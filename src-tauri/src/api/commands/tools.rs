//! Tauri transport adapters. Tool probing and lifecycle policy live in cli.
pub use crate::cli::{
    ToolInstallation, ToolInstallationReport, ToolVersion, WslShellPreferenceInput,
};
use std::collections::HashMap;

#[tauri::command]
pub async fn get_tool_versions(
    tools: Option<Vec<String>>,
    wsl_shell_by_tool: Option<HashMap<String, WslShellPreferenceInput>>,
) -> Result<Vec<ToolVersion>, String> {
    crate::cli::get_tool_versions(tools, wsl_shell_by_tool).await
}

#[tauri::command]
pub async fn run_tool_lifecycle_action(
    tools: Vec<String>,
    action: String,
    wsl_shell_by_tool: Option<HashMap<String, WslShellPreferenceInput>>,
) -> Result<(), String> {
    crate::cli::run_tool_lifecycle_action(tools, action, wsl_shell_by_tool).await
}

#[tauri::command]
pub async fn probe_tool_installations(
    tools: Vec<String>,
) -> Result<Vec<ToolInstallationReport>, String> {
    crate::cli::probe_tool_installations(tools).await
}
