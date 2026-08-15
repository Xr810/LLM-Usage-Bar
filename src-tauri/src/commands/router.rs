//! 本地 router 的 Tauri 命令(T10 建,T16 拆成薄壳)。
//!
//! 这里只做参数搬运:取 state → 调 `crate::api::router::RouterApi` → 把
//! `AppError` 转成 `String`(脱敏面)。业务编排、SQL、校验都在 api 层,
//! 将来的 socket 面板复用 api,而不是这里。
//!
//! 视图/输入结构体已随业务搬进 api 层,这里 `pub use` 转出,
//! 前端看到的类型路径不变。

use crate::api::router::RouterApi;
use crate::store::AppState;

pub use crate::api::router::{
    ModelRouteInput, PointerStateView, RouterProviderInput, RouterProviderView,
    RouterUsageSummaryView, DEFAULT_ROUTER_PORT, POINTER_GAP_KEY,
};

/// 列出全部 router provider(含未启用的,供设置界面编辑)。
#[tauri::command]
pub async fn list_router_providers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RouterProviderView>, String> {
    RouterApi::new(state.db.clone())
        .list_providers()
        .map_err(|e| e.to_string())
}

/// 新增或更新一个 provider。
#[tauri::command]
pub async fn upsert_router_provider(
    state: tauri::State<'_, AppState>,
    input: RouterProviderInput,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .upsert_provider(input)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_router_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .delete_provider(&id)
        .map_err(|e| e.to_string())
}

/// 某个 provider 的模型映射,全量替换。
#[tauri::command]
pub async fn set_model_routes(
    state: tauri::State<'_, AppState>,
    provider_id: String,
    routes: Vec<ModelRouteInput>,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .set_model_routes(&provider_id, routes)
        .map_err(|e| e.to_string())
}

/// 读路由模式。值是 `"auto"` 或 `"manual:<provider_id>"`。
#[tauri::command]
pub async fn get_router_mode(state: tauri::State<'_, AppState>) -> Result<String, String> {
    RouterApi::new(state.db.clone())
        .get_mode()
        .map_err(|e| e.to_string())
}

/// 写路由模式。写侧要严:`manual:<id>` 必须指向存在的 provider。
#[tauri::command]
pub async fn set_router_mode(
    state: tauri::State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    RouterApi::new(state.db.clone())
        .set_mode(&mode)
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
