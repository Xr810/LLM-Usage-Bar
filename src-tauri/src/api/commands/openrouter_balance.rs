//! OpenRouter 账户余额的 Tauri 命令(D1):薄壳,只做参数搬运。
//!
//! 业务编排在 `crate::api::openrouter_balance::OpenRouterBalanceApi`,
//! 这里只取 state → 调 api → 把 AppError 透出(错误串绝不包含 key 明文)。

use crate::api::openrouter_balance::OpenRouterBalanceApi;
use crate::app_state::AppState;
use crate::error::AppError;

pub use crate::services::balance::OpenRouterAccountBalanceView;

/// 设置 OpenRouter 管理 key(trim 后存钥匙串,空串拒绝,不清已有值)。
#[tauri::command]
pub async fn set_openrouter_management_key(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<(), AppError> {
    OpenRouterBalanceApi::new(state.db.clone(), state.credential_store.clone())
        .set_management_key(&key)
}

/// 清除 OpenRouter 管理 key 与余额快照。
#[tauri::command]
pub async fn clear_openrouter_management_key(
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    OpenRouterBalanceApi::new(state.db.clone(), state.credential_store.clone())
        .clear_management_key()
}

/// 读 OpenRouter 账户余额视图(只读 settings/钥匙串,不发请求)。
#[tauri::command]
pub async fn get_openrouter_account_balance(
    state: tauri::State<'_, AppState>,
) -> Result<OpenRouterAccountBalanceView, AppError> {
    OpenRouterBalanceApi::new(state.db.clone(), state.credential_store.clone())
        .get_account_balance()
}
