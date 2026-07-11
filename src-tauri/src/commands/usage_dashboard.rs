use crate::error::AppError;
use crate::store::AppState;
use crate::usage::dashboard::UsageDashboardService;
use crate::usage::domain::{
    RouteBinding, UsageDashboardView, UsageEventPage, UsageProviderInput, UsageProviderView,
};
use crate::usage::quota::QuotaRefreshResult;
use crate::usage::session::ProviderSessionSyncResult;
use tauri::State;

#[tauri::command]
pub fn list_usage_providers(
    state: State<'_, AppState>,
) -> Result<Vec<UsageProviderView>, AppError> {
    list_usage_providers_test_hook(&state)
}

#[tauri::command]
pub fn save_usage_provider(
    state: State<'_, AppState>,
    input: UsageProviderInput,
) -> Result<UsageProviderView, AppError> {
    save_usage_provider_test_hook(&state, input)
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
pub fn get_usage_dashboard(
    state: State<'_, AppState>,
    start_at: i64,
    end_at: i64,
    product_group_id: Option<String>,
) -> Result<UsageDashboardView, AppError> {
    get_usage_dashboard_test_hook(&state, start_at, end_at, product_group_id.as_deref())
}

#[tauri::command]
pub fn get_usage_events(
    state: State<'_, AppState>,
    provider_id: String,
    start_at: i64,
    end_at: i64,
    page: u32,
    page_size: u32,
) -> Result<UsageEventPage, AppError> {
    get_usage_events_test_hook(&state, &provider_id, start_at, end_at, page, page_size)
}

#[tauri::command]
pub async fn refresh_provider_quota(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<QuotaRefreshResult, String> {
    refresh_provider_quota_test_hook(&state, &provider_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn sync_provider_session_usage(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<ProviderSessionSyncResult, AppError> {
    sync_provider_session_usage_test_hook(&state, &provider_id)
}

pub fn list_usage_providers_test_hook(
    state: &AppState,
) -> Result<Vec<UsageProviderView>, AppError> {
    state.db.list_usage_providers()
}

pub fn save_usage_provider_test_hook(
    state: &AppState,
    input: UsageProviderInput,
) -> Result<UsageProviderView, AppError> {
    state.db.save_usage_provider(&input)
}

pub fn set_usage_provider_enabled_test_hook(
    state: &AppState,
    provider_id: &str,
    enabled: bool,
) -> Result<(), AppError> {
    state.db.set_usage_provider_enabled(provider_id, enabled)
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

pub fn get_usage_dashboard_test_hook(
    state: &AppState,
    start_at: i64,
    end_at: i64,
    product_group_id: Option<&str>,
) -> Result<UsageDashboardView, AppError> {
    UsageDashboardService::new(&state.db).get_dashboard(start_at, end_at, product_group_id)
}

pub fn get_usage_events_test_hook(
    state: &AppState,
    provider_id: &str,
    start_at: i64,
    end_at: i64,
    page: u32,
    page_size: u32,
) -> Result<UsageEventPage, AppError> {
    validate_range(start_at, end_at)?;
    if !(1..=200).contains(&page_size) || page == 0 {
        return Err(AppError::Message(
            "page must be at least 1 and page_size must be between 1 and 200".to_string(),
        ));
    }
    if state.db.get_usage_provider(provider_id)?.is_none() {
        return Err(AppError::Message("usage provider not found".to_string()));
    }
    state
        .db
        .list_usage_events(provider_id, start_at, end_at, page, page_size)
}

pub async fn refresh_provider_quota_test_hook(
    state: &AppState,
    provider_id: &str,
) -> Result<QuotaRefreshResult, AppError> {
    state.quota_service.refresh_provider(provider_id).await
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
