use crate::commands::CodexOAuthState;
use crate::credentials::SecretString;
use crate::database::AgentModuleDeleteOutcome;
use crate::error::AppError;
use crate::proxy::{ProxyConfig, ProxyStatus};
use crate::services::SystemProviderConnectionTestResult;
use crate::store::AppState;
use crate::usage::dashboard::UsageDashboardService;
use crate::usage::domain::{
    AgentModuleInput, AgentModuleView, AgentProviderBindingInput, AgentProviderBindingView,
    AgentProxyRouteSetup, AgentProxySetupInfo, LocalBindingKeyReveal,
    ProviderMonitoringDashboardView, RouteBinding, SystemProviderAuthKind,
    UnassignedUsageDiagnostics, UsageDashboardView, UsageEventPage, UsageProviderInput,
    UsageProviderView,
};
use crate::usage::quota::QuotaRefreshResult;
use crate::usage::session::ProviderSessionSyncResult;
use crate::usage::system_providers::{CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID};
use crate::usage::tray_snapshot::TrayUsageSnapshot;
use chrono::{DateTime, Local};
use std::collections::{BTreeMap, BTreeSet};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveUsageProviderCommandInput {
    id: String,
    name: String,
    billing_kind: crate::usage::domain::BillingKind,
    product_group_id: String,
    token_sources: Vec<crate::usage::domain::TokenSource>,
    #[serde(default)]
    session_source_bindings: Option<Vec<String>>,
    quota_source: Option<String>,
    quota_interval_seconds: Option<u64>,
    route_app_type: Option<String>,
    route_config: Option<serde_json::Value>,
    enabled: bool,
}

impl From<SaveUsageProviderCommandInput> for UsageProviderInput {
    fn from(input: SaveUsageProviderCommandInput) -> Self {
        Self {
            id: input.id,
            name: input.name,
            billing_kind: input.billing_kind,
            product_group_id: input.product_group_id,
            token_sources: input.token_sources,
            session_source_bindings: input.session_source_bindings,
            quota_source: input.quota_source,
            quota_interval_seconds: input.quota_interval_seconds,
            route_app_type: input.route_app_type,
            route_config: input.route_config,
            quota_config: None,
            enabled: input.enabled,
        }
    }
}

#[tauri::command]
pub fn list_dashboard_modules(
    state: State<'_, AppState>,
) -> Result<Vec<AgentModuleView>, AppError> {
    list_dashboard_modules_test_hook(&state)
}

#[tauri::command]
pub fn save_dashboard_module(
    state: State<'_, AppState>,
    input: AgentModuleInput,
) -> Result<AgentModuleView, AppError> {
    save_dashboard_module_test_hook(&state, input)
}

#[tauri::command]
pub fn reorder_dashboard_modules(
    state: State<'_, AppState>,
    module_ids: Vec<String>,
) -> Result<Vec<AgentModuleView>, AppError> {
    reorder_dashboard_modules_test_hook(&state, module_ids)
}

#[tauri::command]
pub fn set_dashboard_module_visibility(
    state: State<'_, AppState>,
    module_id: String,
    visible: bool,
) -> Result<AgentModuleView, AppError> {
    set_dashboard_module_visibility_test_hook(&state, &module_id, visible)
}

#[tauri::command]
pub async fn delete_dashboard_module(
    state: State<'_, AppState>,
    module_id: String,
) -> Result<(), AppError> {
    delete_dashboard_module_test_hook(&state, &module_id).await
}

#[tauri::command]
pub async fn list_agent_provider_bindings(
    state: State<'_, AppState>,
    codex_state: State<'_, CodexOAuthState>,
    agent_module_id: Option<String>,
) -> Result<Vec<AgentProviderBindingView>, AppError> {
    let auth = system_provider_auth_snapshot(&state, &codex_state).await;
    list_agent_provider_bindings_with_auth_snapshot_test_hook(
        &state,
        agent_module_id.as_deref(),
        auth,
    )
    .await
}

#[tauri::command]
pub async fn save_agent_provider_binding(
    state: State<'_, AppState>,
    input: AgentProviderBindingInput,
) -> Result<AgentProviderBindingView, AppError> {
    save_agent_provider_binding_test_hook(&state, input).await
}

#[tauri::command]
pub async fn delete_agent_provider_binding(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
) -> Result<(), AppError> {
    delete_agent_provider_binding_test_hook(&state, &binding_id, expected_version).await
}

#[tauri::command]
pub async fn set_agent_provider_binding_api_key(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
    api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError> {
    set_agent_provider_binding_api_key_test_hook(&state, &binding_id, expected_version, api_key)
        .await
}

#[tauri::command]
pub async fn replace_agent_provider_binding_api_key(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
    api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError> {
    replace_agent_provider_binding_api_key_test_hook(&state, &binding_id, expected_version, api_key)
        .await
}

#[tauri::command]
pub async fn clear_agent_provider_binding_api_key(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
) -> Result<AgentProviderBindingView, AppError> {
    clear_agent_provider_binding_api_key_test_hook(&state, &binding_id, expected_version).await
}

#[tauri::command]
pub async fn set_system_provider_api_key(
    state: State<'_, AppState>,
    provider_id: String,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError> {
    set_system_provider_api_key_test_hook(&state, &provider_id, expected_version, api_key).await
}

#[tauri::command]
pub async fn replace_system_provider_api_key(
    state: State<'_, AppState>,
    provider_id: String,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError> {
    replace_system_provider_api_key_test_hook(&state, &provider_id, expected_version, api_key).await
}

#[tauri::command]
pub async fn clear_system_provider_api_key(
    state: State<'_, AppState>,
    provider_id: String,
    expected_version: u64,
) -> Result<UsageProviderView, AppError> {
    clear_system_provider_api_key_test_hook(&state, &provider_id, expected_version).await
}

#[tauri::command]
pub async fn test_system_provider_connection(
    state: State<'_, AppState>,
    provider_id: String,
    expected_version: u64,
) -> Result<SystemProviderConnectionTestResult, AppError> {
    test_system_provider_connection_test_hook(&state, &provider_id, expected_version).await
}

#[tauri::command]
pub async fn reveal_agent_provider_local_key(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError> {
    reveal_agent_provider_local_key_test_hook(&state, &binding_id, expected_version).await
}

#[tauri::command]
pub async fn rotate_agent_provider_local_key(
    state: State<'_, AppState>,
    binding_id: String,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError> {
    rotate_agent_provider_local_key_test_hook(&state, &binding_id, expected_version).await
}

#[tauri::command]
pub async fn get_agent_proxy_setup_info(
    state: State<'_, AppState>,
    agent_module_id: String,
) -> Result<AgentProxySetupInfo, AppError> {
    get_agent_proxy_setup_info_test_hook(&state, &agent_module_id).await
}

#[tauri::command]
pub fn get_unassigned_usage_diagnostics(
    state: State<'_, AppState>,
) -> Result<UnassignedUsageDiagnostics, AppError> {
    get_unassigned_usage_diagnostics_test_hook(&state)
}

#[tauri::command]
pub fn list_usage_providers(
    state: State<'_, AppState>,
) -> Result<Vec<UsageProviderView>, AppError> {
    // Provider-only settings do not publish or act on legacy Agent bindings.
    // Reading their protected status here would open every macOS Keychain item
    // merely because the user opened Settings. Credential verification remains
    // on explicit binding, connection-test, and proxy-use paths.
    state.db.list_usage_providers()
}

#[tauri::command]
pub async fn save_usage_provider(
    state: State<'_, AppState>,
    input: SaveUsageProviderCommandInput,
) -> Result<UsageProviderView, AppError> {
    let provider = state.db.save_usage_provider(&input.into())?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(provider)
}

#[tauri::command]
pub fn delete_usage_provider(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<(), AppError> {
    state.db.delete_usage_provider(&provider_id)?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(())
}

#[tauri::command]
pub fn set_usage_provider_enabled(
    state: State<'_, AppState>,
    provider_id: String,
    enabled: bool,
) -> Result<(), AppError> {
    set_usage_provider_enabled_test_hook(&state, &provider_id, enabled)
}

#[tauri::command]
pub fn get_route_bindings(state: State<'_, AppState>) -> Result<Vec<RouteBinding>, AppError> {
    get_route_bindings_test_hook(&state)
}

#[tauri::command]
pub fn set_route_binding(
    state: State<'_, AppState>,
    protocol: String,
    provider_id: String,
) -> Result<RouteBinding, AppError> {
    set_route_binding_test_hook(&state, &protocol, &provider_id)
}

#[tauri::command]
pub async fn get_usage_dashboard(
    state: State<'_, AppState>,
    start_at: i64,
    end_at: i64,
    agent_module_id: String,
) -> Result<UsageDashboardView, AppError> {
    get_usage_dashboard_test_hook(&state, start_at, end_at, &agent_module_id).await
}

#[tauri::command]
pub fn get_provider_usage_dashboard(
    state: State<'_, AppState>,
    start_at: i64,
    end_at: i64,
) -> Result<ProviderMonitoringDashboardView, AppError> {
    get_provider_usage_dashboard_test_hook(&state, start_at, end_at)
}

#[tauri::command]
pub fn get_usage_events(
    state: State<'_, AppState>,
    agent_module_id: String,
    provider_id: Option<String>,
    start_at: i64,
    end_at: i64,
    page: u64,
    page_size: u64,
) -> Result<UsageEventPage, AppError> {
    get_usage_events_test_hook(
        &state,
        &agent_module_id,
        provider_id.as_deref(),
        start_at,
        end_at,
        page,
        page_size,
    )
}

#[tauri::command]
pub fn get_provider_usage_events(
    state: State<'_, AppState>,
    provider_id: String,
    start_at: i64,
    end_at: i64,
    page: u64,
    page_size: u64,
) -> Result<UsageEventPage, AppError> {
    get_provider_usage_events_test_hook(&state, &provider_id, start_at, end_at, page, page_size)
}

#[tauri::command]
pub async fn refresh_provider_quota(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<QuotaRefreshResult, String> {
    let publisher_app = app.clone();
    refresh_provider_quota_with_sinks_test_hook(
        &state,
        &provider_id,
        move |snapshot| crate::tray_status::publish_tray_usage(&publisher_app, snapshot),
        crate::usage_events::emit_dashboard_invalidated_only,
        Local::now,
    )
    .await
    .map_err(quota_refresh_command_error_payload)
}

fn quota_refresh_command_error_payload(_error: AppError) -> String {
    "quota_refresh_failed".to_string()
}

#[tauri::command]
pub fn sync_provider_session_usage(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<ProviderSessionSyncResult, AppError> {
    sync_provider_session_usage_test_hook(&state, &provider_id)
}

pub fn list_dashboard_modules_test_hook(
    state: &AppState,
) -> Result<Vec<AgentModuleView>, AppError> {
    state.db.list_dashboard_modules()
}

pub fn save_dashboard_module_test_hook(
    state: &AppState,
    input: AgentModuleInput,
) -> Result<AgentModuleView, AppError> {
    save_dashboard_module_with_invalidation_test_hook(
        state,
        input,
        crate::usage_events::notify_dashboard_invalidated,
    )
}

fn save_dashboard_module_with_invalidation_test_hook<I>(
    state: &AppState,
    input: AgentModuleInput,
    invalidate: I,
) -> Result<AgentModuleView, AppError>
where
    I: FnOnce(),
{
    let module = state.db.save_dashboard_module(&input)?;
    invalidate();
    Ok(module)
}

pub fn reorder_dashboard_modules_test_hook(
    state: &AppState,
    module_ids: Vec<String>,
) -> Result<Vec<AgentModuleView>, AppError> {
    reorder_dashboard_modules_with_invalidation_test_hook(
        state,
        module_ids,
        crate::usage_events::notify_dashboard_invalidated,
    )
}

fn reorder_dashboard_modules_with_invalidation_test_hook<I>(
    state: &AppState,
    module_ids: Vec<String>,
    invalidate: I,
) -> Result<Vec<AgentModuleView>, AppError>
where
    I: FnOnce(),
{
    let modules = state.db.reorder_dashboard_modules(&module_ids)?;
    invalidate();
    Ok(modules)
}

pub fn set_dashboard_module_visibility_test_hook(
    state: &AppState,
    module_id: &str,
    visible: bool,
) -> Result<AgentModuleView, AppError> {
    set_dashboard_module_visibility_with_invalidation_test_hook(
        state,
        module_id,
        visible,
        crate::usage_events::notify_dashboard_invalidated,
    )
}

fn set_dashboard_module_visibility_with_invalidation_test_hook<I>(
    state: &AppState,
    module_id: &str,
    visible: bool,
    invalidate: I,
) -> Result<AgentModuleView, AppError>
where
    I: FnOnce(),
{
    let module = state
        .db
        .set_dashboard_module_visibility(module_id, visible)?;
    invalidate();
    Ok(module)
}

pub async fn delete_dashboard_module_test_hook(
    state: &AppState,
    module_id: &str,
) -> Result<(), AppError> {
    delete_dashboard_module_with_invalidation_test_hook(
        state,
        module_id,
        crate::usage_events::notify_dashboard_invalidated,
    )
    .await
}

async fn delete_dashboard_module_with_invalidation_test_hook<I>(
    state: &AppState,
    module_id: &str,
    invalidate: I,
) -> Result<(), AppError>
where
    I: FnOnce(),
{
    let module = state
        .db
        .get_agent_module_including_archived(module_id)?
        .filter(|module| !module.is_fixed)
        .ok_or_else(|| AppError::Message("invalid_agent_module".to_string()))?;
    if module.archived_at.is_some() {
        state.binding_credential_service.reconcile_startup().await?;
    }
    let mut bindings_by_id = state
        .binding_credential_service
        .list_agent_provider_bindings(Some(module_id))
        .await?
        .into_iter()
        .map(|binding| (binding.id.clone(), binding))
        .collect::<BTreeMap<_, _>>();
    let binding_ids = if module.archived_at.is_some() {
        bindings_by_id.keys().cloned().collect::<Vec<_>>()
    } else {
        match state.db.delete_agent_module(module_id)? {
            AgentModuleDeleteOutcome::HardDeleted => Vec::new(),
            AgentModuleDeleteOutcome::Archived { binding_ids } => binding_ids,
        }
    };

    let mut first_error = None;
    if binding_ids
        .iter()
        .any(|binding_id| !bindings_by_id.contains_key(binding_id))
    {
        match state
            .binding_credential_service
            .list_agent_provider_bindings(Some(module_id))
            .await
        {
            Ok(bindings) => {
                for binding in bindings {
                    bindings_by_id.insert(binding.id.clone(), binding);
                }
            }
            Err(error) => {
                first_error = Some(error);
            }
        }
    }
    for binding_id in binding_ids {
        let Some(binding) = bindings_by_id.get(&binding_id) else {
            first_error
                .get_or_insert_with(|| AppError::Message("credential_unavailable".to_string()));
            continue;
        };
        if let Err(error) = state
            .binding_credential_service
            .delete_binding(&binding.id, binding.credential_version)
            .await
        {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    invalidate();
    Ok(())
}

pub async fn list_agent_provider_bindings_test_hook(
    state: &AppState,
    agent_module_id: Option<&str>,
) -> Result<Vec<AgentProviderBindingView>, AppError> {
    list_agent_provider_bindings_with_auth_snapshot_test_hook(
        state,
        agent_module_id,
        SystemProviderAuthSnapshot::default(),
    )
    .await
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemProviderAuthSnapshot {
    pub codex_authenticated: bool,
    pub claude_authenticated: bool,
}

async fn system_provider_auth_snapshot(
    state: &AppState,
    codex_state: &CodexOAuthState,
) -> SystemProviderAuthSnapshot {
    let codex_authenticated = codex_state.0.read().await.is_authenticated().await;
    let claude_authenticated = state.claude_cli_auth_service.status().await.authenticated;
    SystemProviderAuthSnapshot {
        codex_authenticated,
        claude_authenticated,
    }
}

fn project_system_subscription_auth(
    state: &AppState,
    bindings: &mut [AgentProviderBindingView],
    auth: SystemProviderAuthSnapshot,
) -> Result<(), AppError> {
    let provider_enabled = state
        .db
        .list_usage_providers()?
        .into_iter()
        .map(|provider| (provider.id, provider.enabled))
        .collect::<BTreeMap<_, _>>();
    for binding in bindings {
        let authenticated = match binding.provider_id.as_str() {
            CHATGPT_SUBSCRIPTION_ID => Some(auth.codex_authenticated),
            CLAUDE_SUBSCRIPTION_ID => Some(auth.claude_authenticated),
            _ => None,
        };
        if let Some(authenticated) = authenticated {
            binding.effective_enabled = binding.enabled
                && provider_enabled
                    .get(&binding.provider_id)
                    .copied()
                    .unwrap_or(false)
                && authenticated;
        }
    }
    Ok(())
}

pub async fn list_agent_provider_bindings_with_auth_snapshot_test_hook(
    state: &AppState,
    agent_module_id: Option<&str>,
    auth: SystemProviderAuthSnapshot,
) -> Result<Vec<AgentProviderBindingView>, AppError> {
    let mut bindings = state
        .binding_credential_service
        .list_agent_provider_bindings(agent_module_id)
        .await?;
    project_system_subscription_auth(state, &mut bindings, auth)?;
    Ok(bindings)
}

pub async fn save_agent_provider_binding_test_hook(
    state: &AppState,
    input: AgentProviderBindingInput,
) -> Result<AgentProviderBindingView, AppError> {
    let agent_module_id = input.agent_module_id.clone();
    let is_new_fixed_api = input.id.is_none()
        && state
            .db
            .list_usage_providers()?
            .into_iter()
            .find(|provider| provider.id == input.provider_id)
            .is_some_and(|provider| {
                provider.system_auth_kind == Some(SystemProviderAuthKind::ProviderApiKey)
            });
    if is_new_fixed_api {
        let view = state
            .binding_credential_service
            .create_system_api_binding(input)
            .await?;
        crate::usage_events::notify_dashboard_invalidated();
        return Ok(view);
    }
    let saved = state.db.save_agent_provider_binding(&input)?;
    let view = state
        .binding_credential_service
        .list_agent_provider_bindings(Some(&agent_module_id))
        .await?
        .into_iter()
        .find(|binding| binding.id == saved.id)
        .ok_or_else(|| AppError::Message("binding_not_found".to_string()))?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn delete_agent_provider_binding_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
) -> Result<(), AppError> {
    state
        .binding_credential_service
        .delete_binding(binding_id, expected_version)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(())
}

pub async fn set_agent_provider_binding_api_key_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError> {
    let view = state
        .binding_credential_service
        .set_binding_api_key(binding_id, expected_version, api_key)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn replace_agent_provider_binding_api_key_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError> {
    let view = state
        .binding_credential_service
        .replace_binding_api_key(binding_id, expected_version, api_key)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn clear_agent_provider_binding_api_key_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
) -> Result<AgentProviderBindingView, AppError> {
    let view = state
        .binding_credential_service
        .clear_binding_api_key(binding_id, expected_version)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn set_system_provider_api_key_test_hook(
    state: &AppState,
    provider_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError> {
    let view = state
        .binding_credential_service
        .set_provider_api_key(provider_id, expected_version, api_key)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn replace_system_provider_api_key_test_hook(
    state: &AppState,
    provider_id: &str,
    expected_version: u64,
    api_key: SecretString,
) -> Result<UsageProviderView, AppError> {
    let view = state
        .binding_credential_service
        .replace_provider_api_key(provider_id, expected_version, api_key)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn clear_system_provider_api_key_test_hook(
    state: &AppState,
    provider_id: &str,
    expected_version: u64,
) -> Result<UsageProviderView, AppError> {
    let view = state
        .binding_credential_service
        .clear_provider_api_key(provider_id, expected_version)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(view)
}

pub async fn test_system_provider_connection_test_hook(
    state: &AppState,
    provider_id: &str,
    expected_version: u64,
) -> Result<SystemProviderConnectionTestResult, AppError> {
    let result = state
        .system_provider_connection_service
        .test(provider_id, expected_version)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(result)
}

pub async fn reveal_agent_provider_local_key_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError> {
    state
        .binding_credential_service
        .reveal_local_binding_key(binding_id, expected_version)
        .await
}

pub async fn rotate_agent_provider_local_key_test_hook(
    state: &AppState,
    binding_id: &str,
    expected_version: u64,
) -> Result<LocalBindingKeyReveal, AppError> {
    let reveal = state
        .binding_credential_service
        .rotate_local_binding_key(binding_id, expected_version)
        .await?;
    crate::usage_events::notify_dashboard_invalidated();
    Ok(reveal)
}

pub async fn get_agent_proxy_setup_info_test_hook(
    state: &AppState,
    agent_module_id: &str,
) -> Result<AgentProxySetupInfo, AppError> {
    require_active_agent(state, agent_module_id)?;
    let bindings = state
        .binding_credential_service
        .list_agent_provider_bindings(Some(agent_module_id))
        .await?;
    let status = state
        .proxy_service
        .get_status()
        .await
        .map_err(|_| AppError::Message("proxy_status_unavailable".to_string()))?;
    let config = state
        .proxy_service
        .get_config()
        .await
        .map_err(|_| AppError::Message("proxy_status_unavailable".to_string()))?;
    let proxy_origin = reachable_proxy_origin(&status, &config)?;
    let routes = bindings
        .into_iter()
        .map(|binding| -> Result<AgentProxyRouteSetup, AppError> {
            let protocol = binding.route_protocol.clone();
            let publishes_direct_setup = state
                .db
                .agent_provider_binding_supports_direct_api_key(&binding.id)?;
            let (local_base_url, credential_placements) = if publishes_direct_setup {
                (
                    protocol
                        .as_deref()
                        .and_then(|protocol| local_proxy_base_url(&proxy_origin, protocol)),
                    protocol
                        .as_deref()
                        .map(allowed_credential_placements)
                        .unwrap_or_default(),
                )
            } else {
                (None, Vec::new())
            };
            Ok(AgentProxyRouteSetup {
                binding_id: binding.id,
                provider_id: binding.provider_id,
                protocol,
                local_base_url,
                credential_placements,
                credential_status: binding.credential_status,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AgentProxySetupInfo {
        agent_module_id: agent_module_id.to_string(),
        proxy_running: status.running,
        proxy_origin,
        routes,
    })
}

pub fn get_unassigned_usage_diagnostics_test_hook(
    state: &AppState,
) -> Result<UnassignedUsageDiagnostics, AppError> {
    state.db.get_unassigned_usage_diagnostics()
}

async fn verified_bindings_by_provider(
    state: &AppState,
) -> Result<BTreeMap<String, Vec<AgentProviderBindingView>>, AppError> {
    verified_bindings_by_provider_with_auth(state, SystemProviderAuthSnapshot::default()).await
}

async fn verified_bindings_by_provider_with_auth(
    state: &AppState,
    auth: SystemProviderAuthSnapshot,
) -> Result<BTreeMap<String, Vec<AgentProviderBindingView>>, AppError> {
    let mut bindings_by_provider = BTreeMap::new();
    for binding in
        list_agent_provider_bindings_with_auth_snapshot_test_hook(state, None, auth).await?
    {
        bindings_by_provider
            .entry(binding.provider_id.clone())
            .or_insert_with(Vec::new)
            .push(binding);
    }
    Ok(bindings_by_provider)
}

fn hydrate_provider_bindings(
    provider: &mut UsageProviderView,
    bindings_by_provider: &BTreeMap<String, Vec<AgentProviderBindingView>>,
) {
    provider.bindings = bindings_by_provider
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
}

fn hydrate_dashboard_provider_bindings(
    dashboard: &mut UsageDashboardView,
    bindings_by_provider: &BTreeMap<String, Vec<AgentProviderBindingView>>,
) {
    for product in &mut dashboard.product_groups {
        for usage in product
            .subscription_providers
            .iter_mut()
            .chain(product.metered_providers.iter_mut())
        {
            hydrate_provider_bindings(&mut usage.provider, bindings_by_provider);
        }
    }
}

pub async fn list_usage_providers_test_hook(
    state: &AppState,
) -> Result<Vec<UsageProviderView>, AppError> {
    list_usage_providers_with_auth_snapshot_test_hook(state, SystemProviderAuthSnapshot::default())
        .await
}

pub async fn list_usage_providers_with_auth_snapshot_test_hook(
    state: &AppState,
    auth: SystemProviderAuthSnapshot,
) -> Result<Vec<UsageProviderView>, AppError> {
    let bindings_by_provider = verified_bindings_by_provider_with_auth(state, auth).await?;
    let mut providers = state.db.list_usage_providers()?;
    for provider in &mut providers {
        hydrate_provider_bindings(provider, &bindings_by_provider);
    }
    Ok(providers)
}

pub async fn save_usage_provider_test_hook(
    state: &AppState,
    input: UsageProviderInput,
) -> Result<UsageProviderView, AppError> {
    save_usage_provider_with_invalidation_test_hook(
        state,
        input,
        crate::usage_events::notify_dashboard_invalidated,
    )
    .await
}

pub async fn save_usage_provider_command_test_hook(
    state: &AppState,
    input: SaveUsageProviderCommandInput,
) -> Result<UsageProviderView, AppError> {
    save_usage_provider_test_hook(state, input.into()).await
}

async fn save_usage_provider_with_invalidation_test_hook<I>(
    state: &AppState,
    input: UsageProviderInput,
    invalidate: I,
) -> Result<UsageProviderView, AppError>
where
    I: FnOnce(),
{
    let mut provider = state.db.save_usage_provider(&input)?;
    let bindings_by_provider = verified_bindings_by_provider(state).await?;
    hydrate_provider_bindings(&mut provider, &bindings_by_provider);
    invalidate();
    Ok(provider)
}

pub fn set_usage_provider_enabled_test_hook(
    state: &AppState,
    provider_id: &str,
    enabled: bool,
) -> Result<(), AppError> {
    set_usage_provider_enabled_with_invalidation_test_hook(
        state,
        provider_id,
        enabled,
        crate::usage_events::notify_dashboard_invalidated,
    )
}

fn set_usage_provider_enabled_with_invalidation_test_hook<I>(
    state: &AppState,
    provider_id: &str,
    enabled: bool,
    invalidate: I,
) -> Result<(), AppError>
where
    I: FnOnce(),
{
    state.db.set_usage_provider_enabled(provider_id, enabled)?;
    invalidate();
    Ok(())
}

pub fn get_route_bindings_test_hook(state: &AppState) -> Result<Vec<RouteBinding>, AppError> {
    state.db.get_route_bindings()
}

pub fn set_route_binding_test_hook(
    state: &AppState,
    protocol: &str,
    provider_id: &str,
) -> Result<RouteBinding, AppError> {
    if !matches!(protocol, "claude" | "codex" | "gemini") {
        return Err(AppError::Message("unsupported route protocol".to_string()));
    }
    state.db.set_route_binding(protocol, provider_id)
}

pub async fn get_usage_dashboard_test_hook(
    state: &AppState,
    start_at: i64,
    end_at: i64,
    agent_module_id: &str,
) -> Result<UsageDashboardView, AppError> {
    require_active_agent(state, agent_module_id)?;
    let bindings_by_provider = verified_bindings_by_provider(state).await?;
    let mut effective_agents_by_provider: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for binding in bindings_by_provider
        .values()
        .flatten()
        .filter(|binding| binding.effective_enabled)
    {
        effective_agents_by_provider
            .entry(binding.provider_id.clone())
            .or_default()
            .insert(binding.agent_module_id.clone());
    }
    let shared_provider_ids = effective_agents_by_provider
        .into_iter()
        .filter_map(|(provider_id, agent_ids)| (agent_ids.len() > 1).then_some(provider_id));
    let mut dashboard = UsageDashboardService::new(&state.db)
        .with_shared_provider_ids(shared_provider_ids)
        .get_dashboard(start_at, end_at, agent_module_id)?;
    hydrate_dashboard_provider_bindings(&mut dashboard, &bindings_by_provider);
    Ok(dashboard)
}

pub fn get_provider_usage_dashboard_test_hook(
    state: &AppState,
    start_at: i64,
    end_at: i64,
) -> Result<ProviderMonitoringDashboardView, AppError> {
    UsageDashboardService::new(&state.db).get_provider_dashboard(start_at, end_at)
}

pub fn get_usage_events_test_hook(
    state: &AppState,
    agent_module_id: &str,
    provider_id: Option<&str>,
    start_at: i64,
    end_at: i64,
    page: u64,
    page_size: u64,
) -> Result<UsageEventPage, AppError> {
    validate_range(start_at, end_at)?;
    if !(1..=200).contains(&page_size) || page == 0 {
        return Err(AppError::Message(
            "page must be at least 1 and page_size must be between 1 and 200".to_string(),
        ));
    }
    require_active_agent(state, agent_module_id)?;
    if let Some(provider_id) = provider_id {
        if state.db.get_usage_provider(provider_id)?.is_none() {
            return Err(AppError::Message("usage provider not found".to_string()));
        }
    }
    state.db.list_agent_usage_events(
        agent_module_id,
        provider_id,
        start_at,
        end_at,
        page,
        page_size,
    )
}

pub fn get_provider_usage_events_test_hook(
    state: &AppState,
    provider_id: &str,
    start_at: i64,
    end_at: i64,
    page: u64,
    page_size: u64,
) -> Result<UsageEventPage, AppError> {
    validate_range(start_at, end_at)?;
    if state.db.get_usage_provider(provider_id)?.is_none() {
        return Err(AppError::Message("usage provider not found".to_string()));
    }
    let page =
        u32::try_from(page).map_err(|_| AppError::Message("page is too large".to_string()))?;
    let page_size = u32::try_from(page_size)
        .map_err(|_| AppError::Message("page size is too large".to_string()))?;
    state
        .db
        .list_usage_events(provider_id, start_at, end_at, page, page_size)
}

pub async fn refresh_provider_quota_test_hook(
    state: &AppState,
    provider_id: &str,
) -> Result<QuotaRefreshResult, AppError> {
    refresh_provider_quota_with_sinks_test_hook(state, provider_id, |_| {}, || {}, Local::now).await
}

pub(crate) async fn refresh_provider_quota_with_sinks_test_hook<P, I, N>(
    state: &AppState,
    provider_id: &str,
    publish: P,
    invalidate: I,
    completion_now: N,
) -> Result<QuotaRefreshResult, AppError>
where
    P: Fn(&TrayUsageSnapshot) + Send + Sync,
    I: FnOnce(),
    N: FnOnce() -> DateTime<Local>,
{
    match state.quota_service.refresh_provider(provider_id).await {
        Ok(result) => {
            state
                .tray_usage_service
                .rebuild_from_persisted(publish)
                .await;
            invalidate();
            Ok(result)
        }
        Err(error) => {
            state
                .tray_usage_service
                .mark_refresh_failed_at(completion_now().timestamp(), publish)
                .await;
            invalidate();
            Err(error)
        }
    }
}

pub fn sync_provider_session_usage_test_hook(
    state: &AppState,
    provider_id: &str,
) -> Result<ProviderSessionSyncResult, AppError> {
    state.session_usage_service.sync_provider(provider_id)
}

fn validate_range(start_at: i64, end_at: i64) -> Result<(), AppError> {
    if start_at >= end_at {
        return Err(AppError::Message(
            "start_at must be before end_at".to_string(),
        ));
    }
    Ok(())
}

fn require_active_agent(state: &AppState, agent_module_id: &str) -> Result<(), AppError> {
    state
        .db
        .get_agent_module_including_archived(agent_module_id)?
        .filter(|agent| agent.archived_at.is_none())
        .map(|_| ())
        .ok_or_else(|| AppError::Message("invalid_agent_module".to_string()))
}

fn reachable_proxy_origin(status: &ProxyStatus, config: &ProxyConfig) -> Result<String, AppError> {
    let (address, port) = if status.running {
        let address = if !status.address.trim().is_empty() {
            status.address.as_str()
        } else {
            config.listen_address.as_str()
        };
        (address, status.port)
    } else {
        (config.listen_address.as_str(), config.listen_port)
    };
    if port == 0 {
        return Err(AppError::Message("proxy_status_unavailable".to_string()));
    }
    let connect_host = match address {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        address => address,
    };
    let connect_host = if connect_host.contains(':') && !connect_host.starts_with('[') {
        format!("[{connect_host}]")
    } else {
        connect_host.to_string()
    };
    Ok(format!("http://{connect_host}:{port}"))
}

fn local_proxy_base_url(proxy_origin: &str, protocol: &str) -> Option<String> {
    let path = match protocol {
        "claude" => "/claude",
        "codex" => "/codex/v1",
        "opencode" => "/opencode/v1",
        "openclaw" => "/openclaw/v1",
        "hermes" => "/hermes/v1",
        "gemini" => "/gemini",
        "claude-desktop" => "/claude-desktop",
        _ => return None,
    };
    Some(format!("{}{path}", proxy_origin.trim_end_matches('/')))
}

fn allowed_credential_placements(protocol: &str) -> Vec<String> {
    let placements: &[&str] = match protocol {
        "claude" => &["authorization", "x-api-key", "query:key"],
        "codex" | "opencode" | "openclaw" | "hermes" => &["authorization", "query:key"],
        "gemini" => &["authorization", "x-goog-api-key", "query:key"],
        "claude-desktop" => &["x-api-key"],
        _ => &[],
    };
    placements
        .iter()
        .map(|placement| (*placement).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{CredentialStore, CredentialStoreError, SecretString};
    use crate::database::Database;
    use crate::services::subscription::{
        CredentialStatus, QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    };
    use crate::usage::domain::{
        AgentModuleInput, AgentProviderBindingInput, BillingKind, BindingCredentialStatus,
        CostSource, TokenSource, UsageEvent, UsageProviderInput,
    };
    use crate::usage::quota::{QuotaCollector, QuotaService};
    use crate::usage::status::UsageStatus;
    use crate::usage::system_providers::MANAGED_CODEX_QUOTA_SOURCE;
    use chrono::{Local, TimeZone};
    use futures::future::BoxFuture;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MemoryCredentialStore {
        items: Mutex<HashMap<String, Vec<u8>>>,
        delete_attempts: AtomicUsize,
        delete_failures_remaining: AtomicUsize,
    }

    impl CredentialStore for MemoryCredentialStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
            self.items
                .lock()
                .unwrap()
                .insert(slot.to_string(), secret.to_vec());
            Ok(())
        }

        fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
            Ok(self.items.lock().unwrap().get(slot).cloned())
        }

        fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
            self.delete_attempts.fetch_add(1, Ordering::SeqCst);
            if self
                .delete_failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(CredentialStoreError::OperationFailed);
            }
            self.items.lock().unwrap().remove(slot);
            Ok(())
        }
    }

    #[derive(Default)]
    struct ManagedCodexQuotaCollector {
        calls: AtomicUsize,
    }

    impl QuotaCollector for ManagedCodexQuotaCollector {
        fn source(&self) -> &'static str {
            MANAGED_CODEX_QUOTA_SOURCE
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a crate::usage::domain::UsageProviderStored,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(SubscriptionQuota {
                    tool: MANAGED_CODEX_QUOTA_SOURCE.to_string(),
                    credential_status: CredentialStatus::Valid,
                    credential_message: None,
                    success: true,
                    tiers: vec![
                        QuotaTier {
                            name: TIER_FIVE_HOUR.to_string(),
                            utilization: 15.0,
                            resets_at: Some("2026-07-16T10:00:00Z".to_string()),
                            used_value_usd: None,
                            max_value_usd: None,
                        },
                        QuotaTier {
                            name: TIER_SEVEN_DAY.to_string(),
                            utilization: 35.0,
                            resets_at: Some("2026-07-23T00:00:00Z".to_string()),
                            used_value_usd: None,
                            max_value_usd: None,
                        },
                    ],
                    manual_reset_credits: None,
                    extra_usage: None,
                    error: None,
                    queried_at: Some(1),
                })
            })
        }
    }

    const QUOTA_FAILURE_SENTINEL: &str =
        "provider-error-secret-sentinel /Users/example/private/quota.json";

    #[derive(Default)]
    struct CriticalThenFailingQuotaCollector {
        calls: AtomicUsize,
    }

    impl QuotaCollector for CriticalThenFailingQuotaCollector {
        fn source(&self) -> &'static str {
            MANAGED_CODEX_QUOTA_SOURCE
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a crate::usage::domain::UsageProviderStored,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if call > 0 {
                    return Err(QUOTA_FAILURE_SENTINEL.to_string());
                }
                Ok(SubscriptionQuota {
                    tool: MANAGED_CODEX_QUOTA_SOURCE.to_string(),
                    credential_status: CredentialStatus::Valid,
                    credential_message: None,
                    success: true,
                    tiers: vec![
                        QuotaTier {
                            name: TIER_FIVE_HOUR.to_string(),
                            utilization: 95.0,
                            resets_at: Some("2100-01-01T00:00:00Z".to_string()),
                            used_value_usd: None,
                            max_value_usd: None,
                        },
                        QuotaTier {
                            name: TIER_SEVEN_DAY.to_string(),
                            utilization: 95.0,
                            resets_at: Some("2100-01-02T00:00:00Z".to_string()),
                            used_value_usd: None,
                            max_value_usd: None,
                        },
                    ],
                    manual_reset_credits: None,
                    extra_usage: None,
                    error: None,
                    queried_at: Some(1),
                })
            })
        }
    }

    #[tokio::test]
    async fn fixed_chatgpt_refresh_survives_catalog_reconciliation() {
        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let collector = Arc::new(ManagedCodexQuotaCollector::default());
        let quota_service = Arc::new(QuotaService::with_collectors(
            db.clone(),
            vec![collector.clone()],
        ));
        let state = AppState::new_with_credential_store_and_quota_service(
            db.clone(),
            Arc::new(MemoryCredentialStore::default()),
            quota_service,
        );

        let first = refresh_provider_quota_test_hook(&state, CHATGPT_SUBSCRIPTION_ID)
            .await
            .unwrap();
        assert_eq!(
            first.snapshot.five_hour_utilization_percent.as_deref(),
            Some("15")
        );
        assert_eq!(
            first.snapshot.seven_day_utilization_percent.as_deref(),
            Some("35")
        );
        assert!(!first.fetch_state.stale);
        assert!(first.fetch_state.last_error.is_none());
        assert_eq!(
            db.latest_quota_snapshot(CHATGPT_SUBSCRIPTION_ID)
                .unwrap()
                .unwrap()
                .snapshot_id,
            first.snapshot.snapshot_id
        );
        assert_eq!(
            db.get_quota_fetch_state(CHATGPT_SUBSCRIPTION_ID)
                .unwrap()
                .unwrap(),
            first.fetch_state
        );

        db.reconcile_system_providers().unwrap();
        let provider = db
            .get_usage_provider(CHATGPT_SUBSCRIPTION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(
            provider.quota_source.as_deref(),
            Some(MANAGED_CODEX_QUOTA_SOURCE)
        );
        assert_eq!(provider.quota_interval_seconds, Some(300));

        let second = refresh_provider_quota_test_hook(&state, CHATGPT_SUBSCRIPTION_ID)
            .await
            .unwrap();
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
        assert_ne!(second.snapshot.snapshot_id, first.snapshot.snapshot_id);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM quota_snapshots
                     WHERE provider_id = ?1 AND snapshot_id = ?2",
                    rusqlite::params![CHATGPT_SUBSCRIPTION_ID, second.snapshot.snapshot_id],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
        assert!(!second.fetch_state.stale);
        assert!(second.fetch_state.last_error.is_none());
    }

    #[tokio::test]
    async fn subscription_binding_effective_state_follows_transient_official_auth() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let connected = SystemProviderAuthSnapshot {
            codex_authenticated: true,
            claude_authenticated: true,
        };
        let connected_bindings =
            list_agent_provider_bindings_with_auth_snapshot_test_hook(&state, None, connected)
                .await
                .unwrap();
        for provider_id in [CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID] {
            assert!(connected_bindings.iter().any(|binding| {
                binding.provider_id == provider_id && binding.enabled && binding.effective_enabled
            }));
        }

        let disconnected_bindings = list_agent_provider_bindings_with_auth_snapshot_test_hook(
            &state,
            None,
            SystemProviderAuthSnapshot::default(),
        )
        .await
        .unwrap();
        for provider_id in [CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID] {
            assert!(disconnected_bindings.iter().any(|binding| {
                binding.provider_id == provider_id && binding.enabled && !binding.effective_enabled
            }));
        }

        let providers = list_usage_providers_with_auth_snapshot_test_hook(&state, connected)
            .await
            .unwrap();
        assert!(providers
            .iter()
            .find(|provider| provider.id == CHATGPT_SUBSCRIPTION_ID)
            .unwrap()
            .bindings
            .iter()
            .all(|binding| binding.effective_enabled));
    }

    impl MemoryCredentialStore {
        fn item_count(&self) -> usize {
            self.items.lock().unwrap().len()
        }

        fn fail_next_deletes(&self, count: usize) {
            self.delete_failures_remaining
                .store(count, Ordering::SeqCst);
        }

        fn delete_attempt_count(&self) -> usize {
            self.delete_attempts.load(Ordering::SeqCst)
        }
    }

    fn direct_provider(id: &str) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "product".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("claude".to_string()),
            route_config: Some(json!({"baseUrl": "https://upstream.example/secret-path"})),
            quota_config: None,
            enabled: true,
        }
    }

    fn session_provider(id: &str) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "product".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: Some(vec!["claude".to_string(), "codex".to_string()]),
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn managed_provider(id: &str) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "product".to_string(),
            token_sources: vec![TokenSource::SessionLog, TokenSource::Proxy],
            session_source_bindings: Some(vec!["codex".to_string()]),
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("codex".to_string()),
            route_config: Some(json!({
                "baseUrl": "https://chatgpt.com/backend-api",
                "authMode": "oauth"
            })),
            quota_config: None,
            enabled: true,
        }
    }

    const COMMAND_QUOTA_CONFIG_SENTINEL: &str =
        "renderer-command-secret-sentinel /Users/example/private/provider.json";

    fn renderer_provider_json(id: &str) -> serde_json::Value {
        json!({
            "id": id,
            "name": format!("{id} name"),
            "billingKind": "metered",
            "productGroupId": "product",
            "tokenSources": ["proxy"],
            "sessionSourceBindings": null,
            "quotaSource": null,
            "quotaIntervalSeconds": null,
            "routeAppType": "codex",
            "routeConfig": {"baseUrl": "https://upstream.example/v1"},
            "enabled": true
        })
    }

    #[test]
    fn save_usage_provider_command_input_rejects_quota_config_injections_without_echoing_values() {
        for (field, value) in [
            (
                "quotaConfig",
                json!({"accessToken": COMMAND_QUOTA_CONFIG_SENTINEL}),
            ),
            ("quotaConfig", serde_json::Value::Null),
            (
                "quota_config",
                json!({"access_token": COMMAND_QUOTA_CONFIG_SENTINEL}),
            ),
        ] {
            let mut payload = renderer_provider_json("renderer-rejected-secret");
            payload
                .as_object_mut()
                .expect("renderer provider object")
                .insert(field.to_string(), value);

            let error = serde_json::from_value::<SaveUsageProviderCommandInput>(payload)
                .expect_err("quota config fields must be rejected at the command boundary");
            let error = error.to_string();
            assert!(error.contains("unknown field"));
            assert!(!error.contains(COMMAND_QUOTA_CONFIG_SENTINEL));
        }
    }

    #[test]
    fn save_usage_provider_command_input_converts_to_secret_free_domain_input() {
        let input = serde_json::from_value::<SaveUsageProviderCommandInput>(
            renderer_provider_json("renderer-conversion"),
        )
        .unwrap();

        let internal: UsageProviderInput = input.into();

        assert_eq!(internal.id, "renderer-conversion");
        assert_eq!(internal.name, "renderer-conversion name");
        assert_eq!(internal.quota_config, None);
    }

    #[tokio::test]
    async fn save_usage_provider_command_preserves_internal_quota_config_and_redacts_views() {
        let db = Arc::new(Database::memory().unwrap());
        let mut existing = direct_provider("renderer-existing");
        existing.quota_config = Some(json!({
            "accessToken": COMMAND_QUOTA_CONFIG_SENTINEL,
        }));
        db.save_usage_provider(&existing).unwrap();
        let state = AppState::new(db.clone());

        let mut existing_payload = renderer_provider_json("renderer-existing");
        existing_payload["name"] = json!("Renderer metadata edit");
        let existing_input =
            serde_json::from_value::<SaveUsageProviderCommandInput>(existing_payload).unwrap();
        let existing_view = save_usage_provider_command_test_hook(&state, existing_input)
            .await
            .unwrap();

        assert_eq!(existing_view.name, "Renderer metadata edit");
        assert_eq!(
            db.get_usage_provider("renderer-existing")
                .unwrap()
                .unwrap()
                .quota_config,
            existing.quota_config,
        );
        let existing_json = serde_json::to_string(&existing_view).unwrap();
        assert!(!existing_json.contains(COMMAND_QUOTA_CONFIG_SENTINEL));
        assert!(!existing_json.contains("quotaConfig"));
        assert!(!existing_json.contains("quota_config"));

        let created_input = serde_json::from_value::<SaveUsageProviderCommandInput>(
            renderer_provider_json("renderer-new"),
        )
        .unwrap();
        let created_view = save_usage_provider_command_test_hook(&state, created_input)
            .await
            .unwrap();

        assert_eq!(
            db.get_usage_provider("renderer-new")
                .unwrap()
                .unwrap()
                .quota_config,
            None,
        );
        let created_json = serde_json::to_string(&created_view).unwrap();
        assert!(!created_json.contains(COMMAND_QUOTA_CONFIG_SENTINEL));
        assert!(!created_json.contains("quotaConfig"));
        assert!(!created_json.contains("quota_config"));
    }

    #[tokio::test]
    async fn dashboard_module_command_hooks_cover_the_full_crud_contract() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let defaults = list_dashboard_modules_test_hook(&state).unwrap();
        assert_eq!(defaults.len(), 5);

        let created = save_dashboard_module_test_hook(
            &state,
            AgentModuleInput {
                id: None,
                name: "Gemini".to_string(),
                sort_order: 5,
                visible: true,
            },
        )
        .unwrap();

        let ids = vec![
            created.id.clone(),
            "codex".to_string(),
            "claude-code".to_string(),
            "opencode".to_string(),
            "openclaw".to_string(),
            "hermes".to_string(),
        ];
        let reordered = reorder_dashboard_modules_test_hook(&state, ids).unwrap();
        assert_eq!(reordered[0].id, created.id);

        let hidden = set_dashboard_module_visibility_test_hook(&state, &created.id, false).unwrap();
        assert!(!hidden.visible);

        delete_dashboard_module_test_hook(&state, &created.id)
            .await
            .unwrap();
        assert_eq!(list_dashboard_modules_test_hook(&state).unwrap().len(), 5);
    }

    #[tokio::test]
    async fn binding_command_hooks_cover_secret_free_key_lifecycle_and_proxy_setup() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&direct_provider("direct")).unwrap();
        let store = Arc::new(MemoryCredentialStore::default());
        let state = AppState::new_with_credential_store(db, store);

        let binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: "direct".to_string(),
                enabled: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(binding.credential_version, 0);
        assert_eq!(binding.credential_status, BindingCredentialStatus::Missing);
        assert_eq!(
            list_agent_provider_bindings_test_hook(&state, Some("codex"))
                .await
                .unwrap()
                .iter()
                .filter(|candidate| candidate.provider_id == "direct")
                .count(),
            1
        );

        let first_secret = "task-six-first-secret-value";
        let configured = set_agent_provider_binding_api_key_test_hook(
            &state,
            &binding.id,
            0,
            SecretString::new(first_secret.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(configured.credential_version, 1);
        assert_eq!(
            configured.credential_status,
            BindingCredentialStatus::Configured
        );

        let setup = get_agent_proxy_setup_info_test_hook(&state, "codex")
            .await
            .unwrap();
        assert_eq!(setup.agent_module_id, "codex");
        let direct_route = setup
            .routes
            .iter()
            .find(|route| route.provider_id == "direct")
            .expect("direct route");
        assert!(direct_route
            .credential_placements
            .contains(&"authorization".to_string()));
        let setup_json = serde_json::to_string(&setup).unwrap();
        assert!(!setup_json.contains(first_secret));
        assert!(!setup_json.contains("upstream.example"));
        assert!(!setup_json.contains("secret-path"));

        let second_secret = "task-six-second-secret-value";
        let replaced = replace_agent_provider_binding_api_key_test_hook(
            &state,
            &binding.id,
            1,
            SecretString::new(second_secret.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(replaced.credential_version, 2);
        let cleared = clear_agent_provider_binding_api_key_test_hook(&state, &binding.id, 2)
            .await
            .unwrap();
        assert_eq!(cleared.credential_version, 3);
        assert_eq!(cleared.credential_status, BindingCredentialStatus::Missing);

        delete_agent_provider_binding_test_hook(&state, &binding.id, 3)
            .await
            .unwrap();
        assert!(
            !list_agent_provider_bindings_test_hook(&state, Some("codex"))
                .await
                .unwrap()
                .iter()
                .any(|candidate| candidate.provider_id == "direct")
        );
        for public in [
            serde_json::to_string(&configured).unwrap(),
            serde_json::to_string(&replaced).unwrap(),
            serde_json::to_string(&cleared).unwrap(),
        ] {
            assert!(!public.contains(first_secret));
            assert!(!public.contains(second_secret));
        }
    }

    #[tokio::test]
    async fn system_provider_commands_separate_shared_and_local_key_lifecycles() {
        let db = Arc::new(Database::memory().unwrap());
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));
        let binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: "system-openai-api".to_string(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            binding.local_credential_status,
            BindingCredentialStatus::Configured
        );
        assert_eq!(binding.credential_version, 1);
        let local = reveal_agent_provider_local_key_test_hook(
            &state,
            &binding.id,
            binding.credential_version,
        )
        .await
        .unwrap();
        let first_local = local.local_key.clone();

        let provider = set_system_provider_api_key_test_hook(
            &state,
            "system-openai-api",
            0,
            SecretString::new("openai-command-upstream-sentinel".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(provider.upstream_credential_version, 1);
        assert_eq!(
            provider.upstream_credential_status,
            BindingCredentialStatus::Configured
        );
        let public_json =
            serde_json::to_string(&list_usage_providers_test_hook(&state).await.unwrap()).unwrap();
        assert!(!public_json.contains(&first_local));
        assert!(!public_json.contains("openai-command-upstream-sentinel"));

        let rotated = rotate_agent_provider_local_key_test_hook(
            &state,
            &binding.id,
            binding.credential_version,
        )
        .await
        .unwrap();
        assert_ne!(rotated.local_key, first_local);
        assert_eq!(rotated.credential_version, 2);
        assert_eq!(
            reveal_agent_provider_local_key_test_hook(&state, &binding.id, 1)
                .await
                .unwrap_err()
                .to_string(),
            "credential_conflict"
        );

        let cleared = clear_system_provider_api_key_test_hook(
            &state,
            "system-openai-api",
            provider.upstream_credential_version,
        )
        .await
        .unwrap();
        assert_eq!(cleared.upstream_credential_version, 2);
        assert_eq!(
            cleared.upstream_credential_status,
            BindingCredentialStatus::Missing
        );
    }

    #[tokio::test]
    async fn proxy_setup_omits_local_credentials_for_managed_bindings() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&managed_provider("managed-codex"))
            .unwrap();
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));
        let binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: "managed-codex".to_string(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            binding.credential_status,
            BindingCredentialStatus::NotRequired
        );

        let setup = get_agent_proxy_setup_info_test_hook(&state, "codex")
            .await
            .unwrap();
        let managed_route = setup
            .routes
            .iter()
            .find(|route| route.provider_id == "managed-codex")
            .expect("managed route");
        assert_eq!(managed_route.protocol.as_deref(), Some("codex"));
        assert_eq!(managed_route.local_base_url, None);
        assert!(managed_route.credential_placements.is_empty());
    }

    #[tokio::test]
    async fn fixed_openrouter_agents_publish_binding_owned_namespaced_base_urls() {
        let db = Arc::new(Database::memory().unwrap());
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));

        for (agent_module_id, protocol, path) in [
            ("opencode", "opencode", "/opencode/v1"),
            ("openclaw", "openclaw", "/openclaw/v1"),
            ("hermes", "hermes", "/hermes/v1"),
        ] {
            let setup = get_agent_proxy_setup_info_test_hook(&state, agent_module_id)
                .await
                .unwrap();
            let route = setup
                .routes
                .iter()
                .find(|route| route.provider_id == "system-openrouter-api")
                .unwrap();
            assert_eq!(route.protocol.as_deref(), Some(protocol));
            assert_eq!(
                route.local_base_url.as_deref(),
                Some(format!("{}{path}", setup.proxy_origin).as_str())
            );
            assert_eq!(
                route.credential_placements,
                vec!["authorization".to_string(), "query:key".to_string()]
            );
        }
    }

    #[tokio::test]
    async fn proxy_setup_publishes_only_direct_api_key_capabilities() {
        let db = Arc::new(Database::memory().unwrap());
        let mut direct = direct_provider("managed-with-leftover");
        direct.route_app_type = Some("codex".to_string());
        direct.route_config = Some(json!({
            "baseUrl": "https://api.example/v1",
            "authMode": "direct_api_key"
        }));
        db.save_usage_provider(&direct).unwrap();
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));
        let leftover = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: direct.id.clone(),
                enabled: false,
            },
        )
        .await
        .unwrap();
        set_agent_provider_binding_api_key_test_hook(
            &state,
            &leftover.id,
            leftover.credential_version,
            SecretString::new("managed-leftover-key".to_string()),
        )
        .await
        .unwrap();
        state
            .db
            .save_usage_provider(&managed_provider(&direct.id))
            .unwrap();

        let mut unsupported = direct_provider("unsupported-route");
        unsupported.route_app_type = Some("codex".to_string());
        unsupported.route_config = Some(json!({
            "baseUrl": "https://unsupported.example/v1",
            "authMode": "basic"
        }));
        state.db.save_usage_provider(&unsupported).unwrap();
        let unsupported_binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: unsupported.id.clone(),
                enabled: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            unsupported_binding.credential_status,
            BindingCredentialStatus::Unavailable
        );

        let verified = list_agent_provider_bindings_test_hook(&state, Some("codex"))
            .await
            .unwrap();
        let unavailable_custom_bindings = verified
            .iter()
            .filter(|binding| {
                binding.provider_id == direct.id || binding.provider_id == unsupported.id
            })
            .collect::<Vec<_>>();
        assert_eq!(unavailable_custom_bindings.len(), 2);
        assert!(unavailable_custom_bindings
            .iter()
            .all(|binding| { binding.credential_status == BindingCredentialStatus::Unavailable }));

        let setup = get_agent_proxy_setup_info_test_hook(&state, "codex")
            .await
            .unwrap();
        for provider_id in [&direct.id, &unsupported.id] {
            let route = setup
                .routes
                .iter()
                .find(|route| &route.provider_id == provider_id)
                .expect("setup route");
            assert_eq!(route.protocol.as_deref(), Some("codex"));
            assert_eq!(route.local_base_url, None);
            assert!(route.credential_placements.is_empty());
        }
    }

    #[tokio::test]
    async fn provider_commands_embed_verified_binding_views() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&direct_provider("provider-view"))
            .unwrap();
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));
        let binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: "provider-view".to_string(),
                enabled: false,
            },
        )
        .await
        .unwrap();
        set_agent_provider_binding_api_key_test_hook(
            &state,
            &binding.id,
            binding.credential_version,
            SecretString::new("provider-view-protected-key".to_string()),
        )
        .await
        .unwrap();
        let verified = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: Some(binding.id),
                agent_module_id: "codex".to_string(),
                provider_id: "provider-view".to_string(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        assert!(verified.effective_enabled);

        let listed = list_usage_providers_test_hook(&state).await.unwrap();
        assert_eq!(
            listed
                .iter()
                .find(|provider| provider.id == "provider-view")
                .expect("provider view")
                .bindings,
            vec![verified.clone()]
        );

        let mut edited = direct_provider("provider-view");
        edited.name = "Provider view edited".to_string();
        let saved = save_usage_provider_test_hook(&state, edited).await.unwrap();
        assert_eq!(saved.bindings, vec![verified.clone()]);

        let dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "codex")
            .await
            .unwrap();
        assert_eq!(
            dashboard
                .product_groups
                .iter()
                .flat_map(|product| product.metered_providers.iter())
                .find(|usage| usage.provider.id == "provider-view")
                .expect("provider dashboard view")
                .provider
                .bindings,
            vec![verified]
        );
    }

    #[tokio::test]
    async fn proxy_setup_brackets_the_configured_ipv6_origin() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let mut config = state.proxy_service.get_config().await.unwrap();
        config.listen_address = "::1".to_string();
        config.listen_port = 43_123;
        state.proxy_service.update_config(&config).await.unwrap();

        let setup = get_agent_proxy_setup_info_test_hook(&state, "codex")
            .await
            .unwrap();

        assert_eq!(setup.proxy_origin, "http://[::1]:43123");
    }

    #[test]
    fn claude_desktop_binding_key_uses_only_x_api_key() {
        assert_eq!(
            allowed_credential_placements("claude-desktop"),
            vec!["x-api-key".to_string()]
        );
    }

    #[tokio::test]
    async fn proxy_setup_rejects_an_unresolved_ephemeral_port() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let mut config = state.proxy_service.get_config().await.unwrap();
        config.listen_port = 0;
        state.proxy_service.update_config(&config).await.unwrap();

        let error = get_agent_proxy_setup_info_test_hook(&state, "codex")
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "proxy_status_unavailable");
    }

    #[tokio::test]
    async fn deleting_custom_agent_archives_history_and_cleans_protected_bindings() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&direct_provider("direct-delete"))
            .unwrap();
        let custom = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Delete Me".to_string(),
                sort_order: 20,
                visible: true,
            })
            .unwrap();
        let store = Arc::new(MemoryCredentialStore::default());
        let state = AppState::new_with_credential_store(db, store.clone());
        let binding = save_agent_provider_binding_test_hook(
            &state,
            AgentProviderBindingInput {
                id: None,
                agent_module_id: custom.id.clone(),
                provider_id: "direct-delete".to_string(),
                enabled: false,
            },
        )
        .await
        .unwrap();
        set_agent_provider_binding_api_key_test_hook(
            &state,
            &binding.id,
            0,
            SecretString::new("delete-agent-protected-secret".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(store.item_count(), 1);

        delete_dashboard_module_test_hook(&state, &custom.id)
            .await
            .unwrap();

        assert!(state
            .db
            .get_agent_module_including_archived(&custom.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some());
        assert!(
            list_agent_provider_bindings_test_hook(&state, Some(&custom.id))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.item_count(), 0);
    }

    #[tokio::test]
    async fn deleting_custom_agent_attempts_every_binding_cleanup_after_one_failure() {
        let db = Arc::new(Database::memory().unwrap());
        for provider_id in ["delete-first", "delete-second"] {
            db.save_usage_provider(&direct_provider(provider_id))
                .unwrap();
        }
        let custom = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Delete Every Binding".to_string(),
                sort_order: 20,
                visible: true,
            })
            .unwrap();
        let store = Arc::new(MemoryCredentialStore::default());
        let state = AppState::new_with_credential_store(db, store.clone());
        for (provider_id, secret) in [
            ("delete-first", "delete-first-protected-secret"),
            ("delete-second", "delete-second-protected-secret"),
        ] {
            let binding = save_agent_provider_binding_test_hook(
                &state,
                AgentProviderBindingInput {
                    id: None,
                    agent_module_id: custom.id.clone(),
                    provider_id: provider_id.to_string(),
                    enabled: false,
                },
            )
            .await
            .unwrap();
            set_agent_provider_binding_api_key_test_hook(
                &state,
                &binding.id,
                0,
                SecretString::new(secret.to_string()),
            )
            .await
            .unwrap();
        }
        assert_eq!(store.item_count(), 2);
        store.fail_next_deletes(1);

        let error = delete_dashboard_module_test_hook(&state, &custom.id)
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "credential_unavailable");
        assert_eq!(
            store.delete_attempt_count(),
            2,
            "one cleanup failure must not skip the remaining archived binding"
        );
        assert_eq!(
            list_agent_provider_bindings_test_hook(&state, Some(&custom.id))
                .await
                .unwrap()
                .len(),
            1,
            "the failed cleanup remains journaled while the later binding is removed"
        );
        assert_eq!(store.item_count(), 1);

        delete_dashboard_module_test_hook(&state, &custom.id)
            .await
            .unwrap();
        assert!(
            list_agent_provider_bindings_test_hook(&state, Some(&custom.id))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.item_count(), 0);
    }

    #[tokio::test]
    async fn dashboard_marks_verified_multi_agent_provider_shared_without_duplicate_usage() {
        let db = Arc::new(Database::memory().unwrap());
        let mut shared_provider = direct_provider("shared-direct");
        shared_provider.billing_kind = BillingKind::Subscription;
        db.save_usage_provider(&shared_provider).unwrap();
        let state = AppState::new_with_credential_store(
            db.clone(),
            Arc::new(MemoryCredentialStore::default()),
        );
        for (agent_module_id, key) in [
            ("codex", "shared-codex-protected-key"),
            ("claude-code", "shared-claude-protected-key"),
        ] {
            let binding = save_agent_provider_binding_test_hook(
                &state,
                AgentProviderBindingInput {
                    id: None,
                    agent_module_id: agent_module_id.to_string(),
                    provider_id: "shared-direct".to_string(),
                    enabled: false,
                },
            )
            .await
            .unwrap();
            set_agent_provider_binding_api_key_test_hook(
                &state,
                &binding.id,
                0,
                SecretString::new(key.to_string()),
            )
            .await
            .unwrap();
            let enabled = save_agent_provider_binding_test_hook(
                &state,
                AgentProviderBindingInput {
                    id: Some(binding.id),
                    agent_module_id: agent_module_id.to_string(),
                    provider_id: "shared-direct".to_string(),
                    enabled: true,
                },
            )
            .await
            .unwrap();
            assert!(enabled.effective_enabled);
        }
        db.insert_usage_event(&UsageEvent {
            event_id: "shared-codex-event".to_string(),
            source: TokenSource::Proxy,
            provider_id: "shared-direct".to_string(),
            agent_module_id: Some("codex".to_string()),
            product_group_id: "product".to_string(),
            occurred_at: 50,
            model: "model".to_string(),
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: None,
            output_cost_usd: None,
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: None,
            cost_source: CostSource::Unavailable,
            legacy_request_id: None,
            created_at: 50,
        })
        .unwrap();
        db.append_quota_success(&crate::usage::domain::QuotaSnapshot {
            snapshot_id: "shared-direct-quota".to_string(),
            provider_id: "shared-direct".to_string(),
            fetched_at: 60,
            five_hour_utilization_percent: Some("25".to_string()),
            five_hour_resets_at: None,
            seven_day_utilization_percent: Some("50".to_string()),
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: json!({"private": "never-public"}),
            created_at: 60,
        })
        .unwrap();

        let dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "codex")
            .await
            .unwrap();
        let usage = dashboard
            .product_groups
            .iter()
            .flat_map(|product| product.subscription_providers.iter())
            .find(|usage| usage.provider.id == "shared-direct")
            .expect("shared direct usage");
        assert!(usage.shared_account);
        assert_eq!(usage.event_count, 1);
        assert_eq!(usage.input_tokens, 10);
        assert!(usage.quota.is_some());
        assert_eq!(
            dashboard
                .product_groups
                .iter()
                .flat_map(|product| product.subscription_providers.iter())
                .filter(|provider| provider.quota.is_some())
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn dashboard_does_not_mark_session_only_multi_agent_provider_shared() {
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&session_provider("shared-session"))
            .unwrap();
        let state =
            AppState::new_with_credential_store(db, Arc::new(MemoryCredentialStore::default()));
        for agent_module_id in ["codex", "claude-code"] {
            let binding = save_agent_provider_binding_test_hook(
                &state,
                AgentProviderBindingInput {
                    id: None,
                    agent_module_id: agent_module_id.to_string(),
                    provider_id: "shared-session".to_string(),
                    enabled: true,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                binding.credential_status,
                BindingCredentialStatus::NotRequired
            );
            assert!(!binding.effective_enabled);
        }

        let dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "codex")
            .await
            .unwrap();
        let usage = dashboard
            .product_groups
            .iter()
            .flat_map(|product| product.subscription_providers.iter())
            .find(|usage| usage.provider.id == "shared-session")
            .expect("shared session usage");
        assert!(!usage.shared_account);
    }

    #[tokio::test]
    async fn metadata_mutations_invalidate_once_only_after_success() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let invalidations = Arc::new(AtomicUsize::new(0));

        let created = save_dashboard_module_with_invalidation_test_hook(
            &state,
            AgentModuleInput {
                id: None,
                name: "Notification Agent".to_string(),
                sort_order: 5,
                visible: true,
            },
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .unwrap();

        let mut ids = state
            .db
            .list_dashboard_modules()
            .unwrap()
            .into_iter()
            .map(|module| module.id)
            .collect::<Vec<_>>();
        ids.reverse();
        reorder_dashboard_modules_with_invalidation_test_hook(&state, ids, {
            let invalidations = invalidations.clone();
            move || {
                invalidations.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();

        set_dashboard_module_visibility_with_invalidation_test_hook(&state, &created.id, false, {
            let invalidations = invalidations.clone();
            move || {
                invalidations.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();

        save_usage_provider_with_invalidation_test_hook(
            &state,
            direct_provider("metadata-notification-provider"),
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await
        .unwrap();

        set_usage_provider_enabled_with_invalidation_test_hook(
            &state,
            "metadata-notification-provider",
            false,
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .unwrap();

        assert_eq!(invalidations.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn failed_metadata_mutations_do_not_invalidate() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let invalidations = Arc::new(AtomicUsize::new(0));

        let invalidator = || {
            let invalidations = invalidations.clone();
            move || {
                invalidations.fetch_add(1, Ordering::SeqCst);
            }
        };
        save_dashboard_module_with_invalidation_test_hook(
            &state,
            AgentModuleInput {
                id: None,
                name: "  ".to_string(),
                sort_order: 5,
                visible: true,
            },
            invalidator(),
        )
        .unwrap_err();
        reorder_dashboard_modules_with_invalidation_test_hook(
            &state,
            vec!["codex".to_string()],
            invalidator(),
        )
        .unwrap_err();
        set_dashboard_module_visibility_with_invalidation_test_hook(
            &state,
            "missing-agent",
            false,
            invalidator(),
        )
        .unwrap_err();
        save_usage_provider_with_invalidation_test_hook(
            &state,
            direct_provider(CHATGPT_SUBSCRIPTION_ID),
            invalidator(),
        )
        .await
        .unwrap_err();
        set_usage_provider_enabled_with_invalidation_test_hook(
            &state,
            "missing-provider",
            false,
            invalidator(),
        )
        .unwrap_err();

        assert_eq!(invalidations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn custom_agent_delete_invalidates_once_and_failed_delete_does_not() {
        let state = AppState::new(Arc::new(Database::memory().unwrap()));
        let custom = state
            .db
            .save_dashboard_module(&AgentModuleInput {
                id: None,
                name: "Delete Notification Agent".to_string(),
                sort_order: 5,
                visible: true,
            })
            .unwrap();
        let invalidations = Arc::new(AtomicUsize::new(0));

        delete_dashboard_module_with_invalidation_test_hook(&state, &custom.id, {
            let invalidations = invalidations.clone();
            move || {
                invalidations.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();
        assert_eq!(invalidations.load(Ordering::SeqCst), 1);

        delete_dashboard_module_with_invalidation_test_hook(&state, "codex", {
            let invalidations = invalidations.clone();
            move || {
                invalidations.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap_err();
        assert_eq!(invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn manual_provider_quota_success_rebuilds_publishes_and_event_invalidates_once() {
        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let collector = Arc::new(CriticalThenFailingQuotaCollector::default());
        let quota_service = Arc::new(QuotaService::with_collectors(
            db.clone(),
            vec![collector.clone()],
        ));
        let state = AppState::new_with_credential_store_and_quota_service(
            db,
            Arc::new(MemoryCredentialStore::default()),
            quota_service,
        );
        let published = Arc::new(Mutex::new(Vec::new()));
        let invalidations = Arc::new(AtomicUsize::new(0));

        let result = refresh_provider_quota_with_sinks_test_hook(
            &state,
            CHATGPT_SUBSCRIPTION_ID,
            {
                let published = published.clone();
                move |snapshot| published.lock().unwrap().push(snapshot.clone())
            },
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
            || panic!("the completion clock is failure-only"),
        )
        .await
        .unwrap();

        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            result.snapshot.five_hour_utilization_percent.as_deref(),
            Some("95")
        );
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].status, UsageStatus::Red);
        assert!(!published[0].stale);
        assert_eq!(invalidations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn manual_provider_quota_failure_publishes_stale_preserved_alert_and_invalidates_once() {
        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let collector = Arc::new(CriticalThenFailingQuotaCollector::default());
        let quota_service = Arc::new(QuotaService::with_collectors(
            db.clone(),
            vec![collector.clone()],
        ));
        let state = AppState::new_with_credential_store_and_quota_service(
            db,
            Arc::new(MemoryCredentialStore::default()),
            quota_service,
        );
        refresh_provider_quota_test_hook(&state, CHATGPT_SUBSCRIPTION_ID)
            .await
            .unwrap();
        let before = state.tray_usage_service.cached_snapshot().await;
        assert_eq!(before.status, UsageStatus::Red);
        assert!(!before.stale);

        let published = Arc::new(Mutex::new(Vec::new()));
        let invalidations = Arc::new(AtomicUsize::new(0));
        let completion_at = Local.timestamp_opt(2_000_000_000, 0).single().unwrap();
        let error = refresh_provider_quota_with_sinks_test_hook(
            &state,
            CHATGPT_SUBSCRIPTION_ID,
            {
                let published = published.clone();
                move |snapshot| published.lock().unwrap().push(snapshot.clone())
            },
            {
                let invalidations = invalidations.clone();
                move || {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                }
            },
            move || completion_at,
        )
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), QUOTA_FAILURE_SENTINEL);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].status, before.status);
        assert_eq!(published[0].agents, before.agents);
        assert_eq!(published[0].last_success_at, before.last_success_at);
        assert_eq!(published[0].generated_at, completion_at.timestamp());
        assert!(published[0].stale);
        assert_eq!(
            published[0].refresh_error.as_deref(),
            Some("tray_usage_refresh_failed")
        );
        assert_eq!(invalidations.load(Ordering::SeqCst), 1);
        let serialized = serde_json::to_string(&published[0]).unwrap();
        assert!(!serialized.contains(QUOTA_FAILURE_SENTINEL));
        assert!(!serialized.contains("quota_config"));
    }

    #[tokio::test]
    async fn tauri_quota_refresh_error_payload_hides_collector_secret() {
        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let collector = Arc::new(CriticalThenFailingQuotaCollector::default());
        let quota_service = Arc::new(QuotaService::with_collectors(
            db.clone(),
            vec![collector.clone()],
        ));
        let state = AppState::new_with_credential_store_and_quota_service(
            db,
            Arc::new(MemoryCredentialStore::default()),
            quota_service,
        );
        refresh_provider_quota_test_hook(&state, CHATGPT_SUBSCRIPTION_ID)
            .await
            .unwrap();

        let internal_error = refresh_provider_quota_with_sinks_test_hook(
            &state,
            CHATGPT_SUBSCRIPTION_ID,
            |_| {},
            || {},
            Local::now,
        )
        .await
        .unwrap_err();
        assert_eq!(internal_error.to_string(), QUOTA_FAILURE_SENTINEL);

        let command_payload = quota_refresh_command_error_payload(internal_error);
        assert_eq!(command_payload, "quota_refresh_failed");
        assert!(!command_payload.contains(QUOTA_FAILURE_SENTINEL));
    }
}
