//! 本地 router 的 Tauri 命令(T10 建,T16 拆成薄壳)。
//!
//! 这里只做参数搬运:取 state → 调 `crate::api::router::RouterApi` → 把
//! `AppError` 转成 `String`(脱敏面)。业务编排、SQL、校验都在 api 层,
//! 将来的 socket 面板复用 api,而不是这里。
//!
//! 视图/输入结构体已随业务搬进 api 层,这里 `pub use` 转出,
//! 前端看到的类型路径不变。

use crate::api::router::RouterApi;
use crate::app_state::AppState;

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

/// RouterSettings diagnostics; keep provider rows available on Free/community.
/// The timestamp range is half-open and silently clamped for limited users.
#[tauri::command]
pub async fn recent_router_attempts(
    state: tauri::State<'_, AppState>,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<RouterUsageSummaryView>, String> {
    legacy_router_attempts_at(
        &RouterApi::new(state.db.clone()),
        start_at,
        end_at,
        &state.license.status(),
        chrono::Local::now(),
    )
    .map_err(|error| error.to_string())
}

fn legacy_router_attempts_at(
    api: &RouterApi,
    start_at: i64,
    end_at: i64,
    license: &crate::policy::LicenseStatus,
    now: chrono::DateTime<chrono::Local>,
) -> Result<Vec<RouterUsageSummaryView>, crate::error::AppError> {
    let range = crate::usage::trend::clamp_legacy_usage_query(
        Some(start_at),
        Some(end_at),
        false,
        license,
        now,
    )?;
    if range.empty {
        return Ok(Vec::new());
    }
    api.recent_attempts(range.start.unwrap(), range.end.unwrap())
}

#[tauri::command]
pub async fn list_router_model_routes(
    state: tauri::State<'_, AppState>,
    provider_id: String,
) -> Result<Vec<ModelRouteInput>, String> {
    RouterApi::new(state.db.clone())
        .list_model_routes(&provider_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disconnect_router_pointer() -> Result<(), String> {
    crate::route::pointer::disconnect_codex().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{LicenseBuild, LicenseStatus, LicenseTier};
    use crate::store::{AttemptOutcome, Database, RouterAttempt};
    use chrono::{Local, TimeZone};
    use std::sync::Arc;

    #[test]
    fn router_diagnostics_silently_clamp_free_but_preserve_rows_and_pro_bounds() {
        let db = Arc::new(Database::memory().unwrap());
        let now = Local.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let lower = Local
            .with_ymd_and_hms(2026, 9, 27, 0, 0, 0)
            .unwrap()
            .timestamp();
        let upper = Local
            .with_ymd_and_hms(2026, 10, 27, 0, 0, 0)
            .unwrap()
            .timestamp();
        for (started_at, tokens) in [(lower - 1, 100), (lower, 3), (upper - 1, 5), (upper, 200)] {
            db.record_router_attempt(&RouterAttempt {
                started_at,
                logical_model: "model".into(),
                provider_id: "provider".into(),
                outcome: AttemptOutcome::Success,
                failure_kind: None,
                http_status: Some(200),
                input_tokens: Some(tokens),
                output_tokens: Some(tokens * 2),
                duration_ms: Some(1),
            })
            .unwrap();
        }
        let api = RouterApi::new(db);
        for (build, tier) in [
            (LicenseBuild::Community, LicenseTier::Free),
            (LicenseBuild::Official, LicenseTier::Free),
            (LicenseBuild::Community, LicenseTier::Pro),
        ] {
            let mut status = LicenseStatus::community_free();
            status.build = build;
            status.tier = tier;
            let rows = legacy_router_attempts_at(&api, lower - 1, upper + 1, &status, now).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].provider_id, "provider");
            assert_eq!(
                (
                    rows[0].attempts,
                    rows[0].input_tokens,
                    rows[0].output_tokens
                ),
                (2, 8, 16)
            );
            assert!(serde_json::to_value(&rows).unwrap().is_array());
            assert!(
                legacy_router_attempts_at(&api, upper, upper + 1, &status, now)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                legacy_router_attempts_at(&api, lower - 1, lower, &status, now)
                    .unwrap()
                    .is_empty()
            );
        }
        for tier in [LicenseTier::Pro, LicenseTier::ProP3] {
            let mut status = LicenseStatus::community_free();
            status.build = LicenseBuild::Official;
            status.tier = tier;
            let rows = legacy_router_attempts_at(&api, lower - 1, upper + 1, &status, now).unwrap();
            assert_eq!((rows[0].attempts, rows[0].input_tokens), (4, 308));
        }
    }
}
