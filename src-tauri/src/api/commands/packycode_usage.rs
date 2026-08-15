//! PackyCode 账户用量的 Tauri 命令薄壳(D2)。
//!
//! 这里只做参数搬运:取 state → 调 `services::packycode_usage` 的编排 →
//! 返回。业务逻辑、校验、SQL、快照都在 services 模块里,将来的 socket
//! 面板直接复用同一份编排。
//!
//! 安全面:四个命令只暴露视图,绝不回传令牌明文;错误串只含错误码与
//! 状态,不含凭据。

use crate::app_state::AppState;
use crate::error::AppError;
use crate::services::packycode_usage;

pub use crate::services::packycode_usage::PackyCodeAccountUsageView;

/// 保存系统访问令牌 + 用户 ID + base_url。
///
/// 写入前先 `fetch_self` 验证一次,成功才算配好;验证失败不落盘
/// (401/403 报 authentication_failed)。令牌只进钥匙串,不进 settings。
#[tauri::command]
pub async fn set_packycode_usage_credentials(
    state: tauri::State<'_, AppState>,
    access_token: String,
    user_id: String,
    base_url: Option<String>,
) -> Result<(), AppError> {
    packycode_usage::set_credentials(
        state.db.as_ref(),
        state.credential_store.as_ref(),
        access_token,
        user_id,
        base_url,
    )
    .await
}

/// 删钥匙串条目 + 清三个 settings 键。
#[tauri::command]
pub async fn clear_packycode_usage_credentials(
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    packycode_usage::clear_credentials(state.db.as_ref(), state.credential_store.as_ref())
}

/// 只读视图,不发请求。无快照时金额全 None。
#[tauri::command]
pub async fn get_packycode_account_usage(
    state: tauri::State<'_, AppState>,
) -> Result<PackyCodeAccountUsageView, AppError> {
    packycode_usage::account_usage_view(state.db.as_ref(), state.credential_store.as_ref())
}

/// 手动刷新入口:立即抓一次;60 秒内重复调用复用上次快照(防连点)。
#[tauri::command]
pub async fn refresh_packycode_account_usage(
    state: tauri::State<'_, AppState>,
) -> Result<PackyCodeAccountUsageView, AppError> {
    packycode_usage::manual_refresh(state.db.as_ref(), state.credential_store.as_ref()).await
}
