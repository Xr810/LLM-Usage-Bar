//! 本地 router 的 Tauri 命令(T10 建,T16 拆成薄壳)。
//!
//! 这里只做参数搬运:取 state → 调 `crate::api::router::RouterApi` → 把
//! `AppError` 转成 `String`(脱敏面)。业务编排、SQL、校验都在 api 层,
//! socket 面板复用 api,而不是这里。
//!
//! 2026-08-24 起路由按 agent 分,且只存对 `usage_providers` 的引用 ——
//! 所以每个命令都带 `agent`,而「添加 provider」变成了「从候选名单里挑」。
//! 见 docs/design/2026-08-24-router-references-usage-providers.md

use crate::api::router::RouterApi;
use crate::app_state::AppState;

pub use crate::api::router::{
    ModelRouteInput, ModelRouteView, PointerStateView, RouterCandidateView, RouterChainInput,
    RouterProviderView, RouterUsageSummaryView, DEFAULT_ROUTER_PORT, POINTER_GAP_KEY,
};

/// 某个 agent 链上的全部 provider(含未启用的,供设置界面编辑)。
#[tauri::command]
pub async fn list_router_providers(
    state: tauri::State<'_, AppState>,
    agent: String,
) -> Result<Vec<RouterProviderView>, String> {
    RouterApi::new(state.db.clone())
        .list_providers(&agent)
        .map_err(|e| e.to_string())
}

/// 还没加进链、但可以加的那些。
#[tauri::command]
pub async fn list_router_candidates(
    state: tauri::State<'_, AppState>,
    agent: String,
) -> Result<Vec<RouterCandidateView>, String> {
    RouterApi::new(state.db.clone())
        .list_candidates(&agent)
        .map_err(|e| e.to_string())
}

/// 把一家加进链,或改它的顺序 / 启用位。
#[tauri::command]
pub async fn upsert_router_chain_entry(
    state: tauri::State<'_, AppState>,
    agent: String,
    input: RouterChainInput,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .upsert_chain_entry(&agent, input)
        .map_err(|e| e.to_string())
}

/// 把一家移出链。**不动那家 provider 本身。**
#[tauri::command]
pub async fn remove_from_router_chain(
    state: tauri::State<'_, AppState>,
    agent: String,
    provider_id: String,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .remove_from_chain(&agent, &provider_id)
        .map_err(|e| e.to_string())
}

/// 某家的模型映射,全量替换。
#[tauri::command]
pub async fn set_model_routes(
    state: tauri::State<'_, AppState>,
    agent: String,
    provider_id: String,
    routes: Vec<ModelRouteInput>,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .set_model_routes(&agent, &provider_id, routes)
        .map_err(|e| e.to_string())
}

/// 列出这个 agent 的全部模型映射(含已停用那家的),供设置界面渲染。
#[tauri::command]
pub async fn list_model_routes(
    state: tauri::State<'_, AppState>,
    agent: String,
) -> Result<Vec<ModelRouteView>, String> {
    RouterApi::new(state.db.clone())
        .list_model_routes(&agent)
        .map_err(|e| e.to_string())
}

/// 读路由模式。值是 `"auto"` 或 `"manual:<provider_id>"`。
#[tauri::command]
pub async fn get_router_mode(
    state: tauri::State<'_, AppState>,
    agent: String,
) -> Result<String, String> {
    RouterApi::new(state.db.clone())
        .get_mode(&agent)
        .map_err(|e| e.to_string())
}

/// 写路由模式。写侧要严:`manual:<id>` 必须指向存在的 provider。
#[tauri::command]
pub async fn set_router_mode(
    state: tauri::State<'_, AppState>,
    agent: String,
    mode: String,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .set_mode(&agent, &mode)
        .map_err(|e| e.to_string())
}

/// 指针当前状态,供界面显示「已接管 / 未接管 / 读不到」。只读,绝不写。
#[tauri::command]
pub async fn inspect_router_pointer() -> Result<PointerStateView, String> {
    Ok(RouterApi::inspect_pointer())
}

/// 用户显式点「启用」时调用。写前确认 router 在监听、写一次指针、
/// 写后清缺口标记——编排在 api 层,这里只转调。
#[tauri::command]
pub async fn enable_router_pointer(state: tauri::State<'_, AppState>) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .enable_pointer()
        .map_err(|e| e.to_string())
}

/// 最近的路由尝试,按 provider 汇总,供分账面板。时间范围是半开区间。
#[tauri::command]
pub async fn recent_router_attempts(
    state: tauri::State<'_, AppState>,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<RouterUsageSummaryView>, String> {
    RouterApi::new(state.db.clone())
        .recent_attempts(start_at, end_at)
        .map_err(|e| e.to_string())
}
