//! Codex OAuth Authentication Module
//!
//! 实现 OpenAI ChatGPT Plus/Pro 订阅的 OAuth Device Code 流程。
//! 支持多账号管理，每个 Provider 可关联不同的 ChatGPT 账号。
//!
//! ## 认证流程
//! 1. 启动 Device Code 流程，获取 device_auth_id 和 user_code
//! 2. 用户在浏览器中完成 ChatGPT 授权
//! 3. 轮询获取 authorization_code 和 code_verifier（注意：verifier 由服务端返回）
//! 4. 使用 code + verifier 换取 access_token + refresh_token + id_token
//! 5. 自动刷新 access_token（到期前 60 秒）
//!
//! ## 多账号支持
//! - 每个 ChatGPT 账号独立存储 refresh_token
//! - Provider 通过 meta.authBinding 关联账号（auth_provider = "codex_oauth"）
//! - 通过 JWT id_token 提取 chatgpt_account_id 作为账号唯一标识

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// A signed-in ChatGPT account, as reported to the command layer.
///
/// Field names are frozen by the existing `ManagedAuthAccount` mapping.
/// `github_domain` is vestigial for ChatGPT accounts and is always
/// `github.com`; it exists only so both auth providers map uniformly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccount {
    pub id: String,
    pub login: String,
    pub avatar_url: Option<String>,
    pub authenticated_at: i64,
    pub github_domain: String,
}

/// OpenAI device-authorization response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexDeviceCodeResponse {
    /// Device code used for polling.
    pub device_code: String,
    /// User code shown to the person signing in.
    pub user_code: String,
    /// Verification URL.
    pub verification_uri: String,
    /// Expiry, in seconds.
    pub expires_in: u64,
    /// Poll interval, in seconds.
    pub interval: u64,
}

/// OpenAI OAuth 客户端 ID（OpenCode 使用，与官方 Codex CLI 相同）
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Device Code 启动 URL
const DEVICE_AUTH_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";

/// Device Code 轮询 URL
const DEVICE_AUTH_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";

/// OAuth Token URL（用于 code 换 token 和 refresh token）
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Device Code 验证 URL（向用户展示）
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";

/// Device Code 流程的 redirect_uri（OpenAI 服务端约定）
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

/// Token 刷新提前量（毫秒）
const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;

/// Device Code 默认有效时长（秒），OpenAI 文档约定 15 分钟
const DEVICE_CODE_DEFAULT_EXPIRES_IN: u64 = 900;

/// 轮询间隔安全余量（秒）
const POLLING_SAFETY_MARGIN_SECS: u64 = 3;

/// Historical User-Agent retained for OAuth server compatibility.
const CODEX_USER_AGENT: &str = crate::product_identity::LEGACY_CODEX_OAUTH_USER_AGENT;
const TOKEN_REFRESH_LOG_MESSAGE: &str = "[CodexOAuth] access_token refresh required";

/// Codex OAuth 错误
#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("等待用户授权中")]
    AuthorizationPending,

    #[error("用户拒绝授权")]
    AccessDenied,

    #[error("Device Code 已过期")]
    ExpiredToken,

    #[error("OAuth Token 获取失败: {0}")]
    TokenFetchFailed(String),

    #[error("Refresh Token 失效或已过期")]
    RefreshTokenInvalid,

    #[error("网络错误: {0}")]
    NetworkError(String),

    #[error("解析错误: {0}")]
    ParseError(String),

    #[error("IO 错误: {0}")]
    IoError(String),

    #[error("账号不存在: {0}")]
    AccountNotFound(String),
}

impl From<reqwest::Error> for CodexOAuthError {
    fn from(err: reqwest::Error) -> Self {
        CodexOAuthError::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CodexOAuthError {
    fn from(err: std::io::Error) -> Self {
        CodexOAuthError::IoError(err.to_string())
    }
}

/// OpenAI Device Code 响应
#[derive(Debug, Clone, Deserialize)]
struct DeviceCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<serde_json::Value>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// OpenAI Device Code 轮询响应（成功）
#[derive(Debug, Clone, Deserialize)]
struct DevicePollSuccess {
    authorization_code: String,
    code_verifier: String,
}

/// OAuth Token 响应
#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// 解析后的 JWT claims（仅关心 chatgpt_account_id 等字段）
#[derive(Debug, Clone, Default, Deserialize)]
struct IdTokenClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    organizations: Vec<OrgClaim>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    openai_auth: Option<OpenAiAuthClaim>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OrgClaim {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenAiAuthClaim {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    /// Unix seconds. OpenAI has been observed to send this as a number and as
    /// a numeric string; accept both rather than silently dropping one.
    #[serde(default)]
    chatgpt_subscription_active_until: Option<serde_json::Value>,
}

impl OpenAiAuthClaim {
    fn subscription_active_until(&self) -> Option<i64> {
        match self.chatgpt_subscription_active_until.as_ref()? {
            serde_json::Value::Number(value) => value.as_i64(),
            serde_json::Value::String(value) => value.trim().parse::<i64>().ok(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CodexSubscriptionMetadata {
    plan_type: Option<String>,
    plan_renews_at: Option<i64>,
}

/// 缓存的 access_token（含过期时间）
#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    /// 过期时间戳（毫秒）
    expires_at_ms: i64,
}

impl CachedAccessToken {
    fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        self.expires_at_ms - now < TOKEN_REFRESH_BUFFER_MS
    }
}

/// 进行中的 Device Code 条目，带过期时间以便清理放弃的登录流程
#[derive(Debug, Clone)]
struct PendingDeviceCode {
    user_code: String,
    /// Unix 毫秒时间戳，超时后可清理
    expires_at_ms: i64,
}

/// 持久化的账号数据
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexAccountData {
    /// chatgpt_account_id（同时作为 HashMap 的 key）
    pub account_id: String,
    /// 账号邮箱（如果可获取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Refresh Token（持久化）
    pub refresh_token: String,
    /// 认证时间戳（秒）
    pub authenticated_at: i64,
    /// OAuth id_token 中经过标准化的 ChatGPT 方案类型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// 当前订阅周期的结束时间（Unix 秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_renews_at: Option<i64>,
}

/// 公开的账号信息（返回给前端，复用 CodexAccount 结构）
impl From<&CodexAccountData> for CodexAccount {
    fn from(data: &CodexAccountData) -> Self {
        CodexAccount {
            id: data.account_id.clone(),
            // 用 email 作为显示名（若无则用 account_id）
            login: data
                .email
                .clone()
                .unwrap_or_else(|| format!("ChatGPT ({})", &data.account_id)),
            avatar_url: None,
            authenticated_at: data.authenticated_at,
            github_domain: "github.com".to_string(),
        }
    }
}

/// 持久化存储结构（v1）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexOAuthStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: HashMap<String, CodexAccountData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}

#[cfg(test)]
type TestRefreshFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<OAuthTokenResponse, CodexOAuthError>>
            + Send
            + 'static,
    >,
>;

#[cfg(test)]
type TestRefreshHook = Arc<dyn Fn(String) -> TestRefreshFuture + Send + Sync>;

#[cfg(test)]
type TestPersistenceFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

#[cfg(test)]
type TestPersistenceHook = Arc<dyn Fn() -> TestPersistenceFuture + Send + Sync>;

/// Codex OAuth 认证管理器（多账号）
///
/// Clone 会共享所有内部状态锁，仅复制不可变配置。这允许调用方在
/// 释放包装本 manager 的外层锁后，继续执行可能包含网络等待的操作。
#[derive(Clone)]
pub struct CodexOAuthManager {
    accounts: Arc<RwLock<HashMap<String, CodexAccountData>>>,
    default_account_id: Arc<RwLock<Option<String>>>,
    /// 内存缓存的 access_token（不持久化）
    access_tokens: Arc<RwLock<HashMap<String, CachedAccessToken>>>,
    /// 每个账号的刷新锁
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// 进行中的 Device Code 流程：device_auth_id -> {user_code, expires_at_ms}
    /// 过期条目会在 start_device_flow 时被清理，防止放弃的登录流程导致无界增长
    pending_device_codes: Arc<RwLock<HashMap<String, PendingDeviceCode>>>,
    /// 账号状态变更与对应的磁盘快照必须在同一临界区内完成，防止旧快照覆盖删除/登出。
    persistence_lock: Arc<Mutex<()>>,
    storage_path: PathBuf,
    #[cfg(test)]
    refresh_hook: Arc<RwLock<Option<TestRefreshHook>>>,
    #[cfg(test)]
    persistence_snapshot_hook: Arc<RwLock<Option<TestPersistenceHook>>>,
}

impl CodexOAuthManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("codex_oauth_auth.json");

        let manager = Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            pending_device_codes: Arc::new(RwLock::new(HashMap::new())),
            persistence_lock: Arc::new(Mutex::new(())),
            storage_path,
            #[cfg(test)]
            refresh_hook: Arc::new(RwLock::new(None)),
            #[cfg(test)]
            persistence_snapshot_hook: Arc::new(RwLock::new(None)),
        };

        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[CodexOAuth] 加载存储失败: {e}");
        }

        manager
    }

    // ==================== 设备码流程 ====================

    /// 启动 Device Code 流程
    ///
    /// 返回 CodexDeviceCodeResponse，字段含义对应 OpenAI 的字段：
    /// - device_code = device_auth_id
    /// - user_code = user_code
    /// - verification_uri = https://auth.openai.com/codex/device
    pub async fn start_device_flow(&self) -> Result<CodexDeviceCodeResponse, CodexOAuthError> {
        log::info!("[CodexOAuth] 启动 Device Code 流程");

        let response = crate::http_client::get()
            .post(DEVICE_AUTH_USERCODE_URL)
            .header("Content-Type", "application/json")
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&serde_json::json!({ "client_id": CODEX_CLIENT_ID }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::NetworkError(format!(
                "Device Code 请求失败: {status} - {text}"
            )));
        }

        let device: DeviceCodeResponse = response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        let interval = parse_interval(device.interval.as_ref());
        let expires_in = device.expires_in.unwrap_or(DEVICE_CODE_DEFAULT_EXPIRES_IN);
        let expires_at_ms = chrono::Utc::now().timestamp_millis() + (expires_in as i64) * 1000;

        // 记录 device_auth_id -> 用户码映射；同时清理所有已过期的条目，
        // 避免用户放弃登录流程导致 HashMap 无界增长
        {
            let mut pending = self.pending_device_codes.write().await;
            let now_ms = chrono::Utc::now().timestamp_millis();
            pending.retain(|_, entry| entry.expires_at_ms > now_ms);
            pending.insert(
                device.device_auth_id.clone(),
                PendingDeviceCode {
                    user_code: device.user_code.clone(),
                    expires_at_ms,
                },
            );
        }

        log::info!(
            "[CodexOAuth] 获取 Device Code 成功，user_code: {}",
            device.user_code
        );

        Ok(CodexDeviceCodeResponse {
            device_code: device.device_auth_id,
            user_code: device.user_code,
            verification_uri: DEVICE_VERIFICATION_URL.to_string(),
            expires_in,
            interval,
        })
    }

    /// 轮询 Device Code 状态
    ///
    /// 接收 device_code（即 device_auth_id），返回 Some(account) 表示授权成功
    pub async fn poll_for_token(
        &self,
        device_code: &str,
    ) -> Result<Option<CodexAccount>, CodexOAuthError> {
        let entry = {
            let pending = self.pending_device_codes.read().await;
            pending.get(device_code).cloned()
        };

        let entry = entry.ok_or_else(|| {
            CodexOAuthError::TokenFetchFailed(
                "未找到对应的 user_code，请重新启动登录流程".to_string(),
            )
        })?;

        if entry.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            let mut pending = self.pending_device_codes.write().await;
            pending.remove(device_code);
            return Err(CodexOAuthError::ExpiredToken);
        }

        let user_code = entry.user_code;

        log::debug!("[CodexOAuth] 轮询 Device Code");

        let poll_response = crate::http_client::get()
            .post(DEVICE_AUTH_TOKEN_URL)
            .header("Content-Type", "application/json")
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&serde_json::json!({
                "device_auth_id": device_code,
                "user_code": user_code,
            }))
            .send()
            .await?;

        let status = poll_response.status();

        // 403/404 表示用户未完成授权，继续轮询
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
            return Err(CodexOAuthError::AuthorizationPending);
        }

        if status == reqwest::StatusCode::GONE {
            return Err(CodexOAuthError::ExpiredToken);
        }

        if !status.is_success() {
            let text = poll_response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "{status} - {text}"
            )));
        }

        let success: DevicePollSuccess = poll_response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        log::info!("[CodexOAuth] 用户已授权，正在换取 OAuth Token");

        // 用 authorization_code + code_verifier 换 token
        let tokens = self
            .exchange_code_for_tokens(&success.authorization_code, &success.code_verifier)
            .await?;

        // 清理 pending device code
        {
            let mut pending = self.pending_device_codes.write().await;
            pending.remove(device_code);
        }

        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            CodexOAuthError::TokenFetchFailed("响应缺少 refresh_token".to_string())
        })?;

        let (account_id, email) = extract_identity_from_tokens(&tokens);
        let subscription_metadata =
            extract_subscription_metadata_from_tokens(&tokens).unwrap_or_default();
        let account_id = account_id.ok_or_else(|| {
            CodexOAuthError::ParseError("无法从 token 中提取 account_id".to_string())
        })?;

        let account = self
            .add_account_with_access_token_internal(
                account_id,
                refresh_token,
                email,
                Some(CachedAccessToken {
                    token: tokens.access_token,
                    expires_at_ms: compute_expires_at_ms(tokens.expires_in),
                }),
                subscription_metadata,
            )
            .await?;

        Ok(Some(account))
    }

    /// 用 authorization_code + code_verifier 换取 tokens
    async fn exchange_code_for_tokens(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let response = crate::http_client::get()
            .post(OAUTH_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", CODEX_USER_AGENT)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", DEVICE_REDIRECT_URI),
                ("client_id", CODEX_CLIENT_ID),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "Token 交换失败: {status} - {text}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))
    }

    /// 用 refresh_token 刷新 access_token
    async fn refresh_with_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        #[cfg(test)]
        {
            let hook = self.refresh_hook.read().await.clone();
            if let Some(hook) = hook {
                return hook(refresh_token.to_string()).await;
            }
        }

        let response = crate::http_client::get()
            .post(OAUTH_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", CODEX_USER_AGENT)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CODEX_CLIENT_ID),
                ("scope", "openid profile email"),
            ])
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(CodexOAuthError::RefreshTokenInvalid);
        }

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "Refresh 失败: {status} - {text}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))
    }

    // ==================== Token 获取（含自动刷新） ====================

    /// 获取指定账号的有效 access_token（必要时自动刷新）
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        // accounts -> access_tokens 是账号删除、清空和刷新提交的统一锁序。
        // 这也保证缓存快速路径不会在账号已删除后继续返回 token。
        if let Some(token) = self.cached_token_for_existing_account(account_id).await? {
            return Ok(token);
        }

        log::info!("{TOKEN_REFRESH_LOG_MESSAGE}");

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;

        // double-check
        if let Some(token) = self.cached_token_for_existing_account(account_id).await? {
            return Ok(token);
        }

        let refresh_token = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|a| a.refresh_token.clone())
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?
        };

        let new_tokens = self.refresh_with_token(&refresh_token).await?;

        let subscription_metadata = extract_subscription_metadata_from_tokens(&new_tokens);
        let access_token = new_tokens.access_token.clone();
        let expires_at_ms = compute_expires_at_ms(new_tokens.expires_in);
        let mut account_data_changed = false;
        let persistence_guard = self.persistence_lock.lock().await;

        {
            // 与 remove_account/clear_auth 使用同一 accounts -> access_tokens
            // 锁序提交网络刷新结果。删除先线性化时，这里必须失败；
            // 刷新先线性化时，后续删除会同时清掉账号和 token。
            let mut accounts = self.accounts.write().await;
            let account = accounts
                .get_mut(account_id)
                .filter(|account| account.refresh_token == refresh_token)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
            let mut tokens = self.access_tokens.write().await;

            if let Some(new_refresh) = new_tokens.refresh_token {
                if new_refresh != refresh_token {
                    account.refresh_token = new_refresh;
                    account_data_changed = true;
                }
            }

            if let Some(metadata) = subscription_metadata {
                if account.plan_type != metadata.plan_type {
                    account.plan_type = metadata.plan_type;
                    account_data_changed = true;
                }
                if account.plan_renews_at != metadata.plan_renews_at {
                    account.plan_renews_at = metadata.plan_renews_at;
                    account_data_changed = true;
                }
            }

            tokens.insert(
                account_id.to_string(),
                CachedAccessToken {
                    token: access_token.clone(),
                    expires_at_ms,
                },
            );
        }

        if account_data_changed {
            self.persist_current_state_locked(&persistence_guard)
                .await?;
        }

        Ok(access_token)
    }

    /// 获取默认账号的有效 token
    pub async fn get_valid_token(&self) -> Result<String, CodexOAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.get_valid_token_for_account(&id).await,
            None => Err(CodexOAuthError::AccountNotFound(
                "无可用的 ChatGPT 账号".to_string(),
            )),
        }
    }

    /// Return the valid token plus the only non-secret id_token metadata used
    /// by the quota dashboard. The raw id_token never leaves token parsing.
    pub(crate) async fn get_valid_token_and_subscription_for_account(
        &self,
        account_id: &str,
    ) -> Result<(String, Option<String>, Option<i64>), CodexOAuthError> {
        let token = self.get_valid_token_for_account(account_id).await?;
        let accounts = self.accounts.read().await;
        let account = accounts
            .get(account_id)
            .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
        Ok((token, account.plan_type.clone(), account.plan_renews_at))
    }

    /// 获取默认账号 ID（热路径使用，避免克隆整个账号 HashMap）
    pub async fn default_account_id(&self) -> Option<String> {
        self.resolve_default_account_id().await
    }

    // ==================== 多账号管理 ====================

    pub async fn list_accounts(&self) -> Vec<CodexAccount> {
        let accounts = self.accounts.read().await.clone();
        let default_id = self.resolve_default_account_id().await;
        Self::sorted_accounts(&accounts, default_id.as_deref())
    }

    pub async fn remove_account(&self, account_id: &str) -> Result<(), CodexOAuthError> {
        log::info!("[CodexOAuth] 移除账号: {account_id}");
        let persistence_guard = self.persistence_lock.lock().await;

        {
            let mut accounts = self.accounts.write().await;
            let mut tokens = self.access_tokens.write().await;
            if accounts.remove(account_id).is_none() {
                return Err(CodexOAuthError::AccountNotFound(account_id.to_string()));
            }
            tokens.remove(account_id);
        }
        {
            let mut locks = self.refresh_locks.write().await;
            locks.remove(account_id);
        }

        {
            let accounts = self.accounts.read().await;
            let mut default = self.default_account_id.write().await;
            if default.as_deref() == Some(account_id) {
                *default = Self::fallback_default_account_id(&accounts);
            }
        }

        self.persist_current_state_locked(&persistence_guard)
            .await?;
        Ok(())
    }

    pub async fn set_default_account(&self, account_id: &str) -> Result<(), CodexOAuthError> {
        let persistence_guard = self.persistence_lock.lock().await;

        {
            let accounts = self.accounts.read().await;
            if !accounts.contains_key(account_id) {
                return Err(CodexOAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        {
            let mut default = self.default_account_id.write().await;
            *default = Some(account_id.to_string());
        }

        self.persist_current_state_locked(&persistence_guard)
            .await?;
        Ok(())
    }

    pub async fn clear_auth(&self) -> Result<(), CodexOAuthError> {
        log::info!("[CodexOAuth] 清除所有认证");
        let persistence_guard = self.persistence_lock.lock().await;

        {
            let mut accounts = self.accounts.write().await;
            let mut tokens = self.access_tokens.write().await;
            accounts.clear();
            tokens.clear();
        }
        {
            let mut default = self.default_account_id.write().await;
            *default = None;
        }
        {
            let mut locks = self.refresh_locks.write().await;
            locks.clear();
        }
        {
            let mut pending = self.pending_device_codes.write().await;
            pending.clear();
        }

        self.persist_current_state_locked(&persistence_guard)
            .await?;
        Ok(())
    }

    pub async fn is_authenticated(&self) -> bool {
        let accounts = self.accounts.read().await;
        !accounts.is_empty()
    }

    /// 获取认证状态摘要（与 Copilot 的格式保持一致，便于复用前端）
    pub async fn get_status(&self) -> CodexOAuthStatus {
        let accounts_map = self.accounts.read().await.clone();
        let default_id = self.resolve_default_account_id().await;
        let account_list = Self::sorted_accounts(&accounts_map, default_id.as_deref());
        let authenticated = !account_list.is_empty();
        let username = default_id
            .as_ref()
            .and_then(|id| accounts_map.get(id))
            .and_then(|a| a.email.clone())
            .or_else(|| account_list.first().map(|a| a.login.clone()));

        CodexOAuthStatus {
            accounts: account_list,
            default_account_id: default_id,
            authenticated,
            username,
        }
    }

    // ==================== 内部方法 ====================

    #[cfg(test)]
    async fn add_account_internal(
        &self,
        account_id: String,
        refresh_token: String,
        email: Option<String>,
    ) -> Result<CodexAccount, CodexOAuthError> {
        self.add_account_with_access_token_internal(
            account_id,
            refresh_token,
            email,
            None,
            CodexSubscriptionMetadata::default(),
        )
        .await
    }

    async fn add_account_with_access_token_internal(
        &self,
        account_id: String,
        refresh_token: String,
        email: Option<String>,
        access_token: Option<CachedAccessToken>,
        subscription_metadata: CodexSubscriptionMetadata,
    ) -> Result<CodexAccount, CodexOAuthError> {
        let now = chrono::Utc::now().timestamp();
        let persistence_guard = self.persistence_lock.lock().await;

        let data = CodexAccountData {
            account_id: account_id.clone(),
            email,
            refresh_token,
            authenticated_at: now,
            plan_type: subscription_metadata.plan_type,
            plan_renews_at: subscription_metadata.plan_renews_at,
        };

        let account = CodexAccount::from(&data);

        {
            let mut accounts = self.accounts.write().await;
            accounts.insert(account_id.clone(), data);
        }

        if let Some(access_token) = access_token {
            let mut tokens = self.access_tokens.write().await;
            tokens.insert(account_id.clone(), access_token);
        }

        {
            let mut default = self.default_account_id.write().await;
            if default.is_none() {
                *default = Some(account_id);
            }
        }

        self.persist_current_state_locked(&persistence_guard)
            .await?;
        Ok(account)
    }

    fn fallback_default_account_id(accounts: &HashMap<String, CodexAccountData>) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, a), (id_b, b)| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id.clone())
    }

    fn sorted_accounts(
        accounts: &HashMap<String, CodexAccountData>,
        default_account_id: Option<&str>,
    ) -> Vec<CodexAccount> {
        let mut list: Vec<CodexAccount> = accounts.values().map(CodexAccount::from).collect();
        list.sort_by(|a, b| {
            let a_default = default_account_id == Some(a.id.as_str());
            let b_default = default_account_id == Some(b.id.as_str());
            b_default
                .cmp(&a_default)
                .then_with(|| b.authenticated_at.cmp(&a.authenticated_at))
                .then_with(|| a.login.cmp(&b.login))
        });
        list
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;

        if let Some(id) = stored {
            if accounts.contains_key(&id) {
                return Some(id);
            }
        }

        Self::fallback_default_account_id(&accounts)
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let locks = self.refresh_locks.read().await;
            if let Some(lock) = locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.refresh_locks.write().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    async fn cached_token_for_existing_account(
        &self,
        account_id: &str,
    ) -> Result<Option<String>, CodexOAuthError> {
        let accounts = self.accounts.read().await;
        if !accounts.contains_key(account_id) {
            return Err(CodexOAuthError::AccountNotFound(account_id.to_string()));
        }

        let tokens = self.access_tokens.read().await;
        Ok(tokens
            .get(account_id)
            .filter(|cached| !cached.is_expiring_soon())
            .map(|cached| cached.token.clone()))
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CodexOAuthError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储路径".to_string()))?;
        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储文件名".to_string()))?
            .to_string_lossy()
            .to_string();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!("{file_name}.tmp.{ts}"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            fs::rename(&tmp_path, &self.storage_path)?;
            fs::set_permissions(&self.storage_path, fs::Permissions::from_mode(0o600))?;
        }

        #[cfg(windows)]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                let _ = fs::remove_file(&self.storage_path);
            }
            fs::rename(&tmp_path, &self.storage_path)?;
        }

        Ok(())
    }

    fn load_from_disk_sync(&self) -> Result<(), CodexOAuthError> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)?;
        let store: CodexOAuthStore = serde_json::from_str(&content)
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        if let Ok(mut accounts) = self.accounts.try_write() {
            *accounts = store.accounts;
            log::info!("[CodexOAuth] 从磁盘加载 {} 个账号", accounts.len());
        }
        if let Ok(mut default) = self.default_account_id.try_write() {
            *default = store.default_account_id;
            if default.is_none() {
                if let Ok(accounts) = self.accounts.try_read() {
                    *default = Self::fallback_default_account_id(&accounts);
                }
            }
        }

        Ok(())
    }

    async fn persist_current_state_locked(
        &self,
        _persistence_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), CodexOAuthError> {
        let accounts = self.accounts.read().await.clone();
        let default = self.resolve_default_account_id().await;

        let store = CodexOAuthStore {
            version: 1,
            accounts,
            default_account_id: default,
        };

        let content = serde_json::to_string_pretty(&store)
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        #[cfg(test)]
        {
            let hook = self.persistence_snapshot_hook.read().await.clone();
            if let Some(hook) = hook {
                hook().await;
            }
        }

        if store.accounts.is_empty() {
            match fs::remove_file(&self.storage_path) {
                Ok(()) => log::info!("[CodexOAuth] 已删除空的账号存储"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else {
            self.write_store_atomic(&content)?;
        }

        log::info!(
            "[CodexOAuth] 保存到磁盘成功（{} 个账号）",
            store.accounts.len()
        );

        Ok(())
    }
}

/// Codex OAuth 状态摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexOAuthStatus {
    pub accounts: Vec<CodexAccount>,
    pub default_account_id: Option<String>,
    pub authenticated: bool,
    pub username: Option<String>,
}

// ==================== 工具函数 ====================

/// 解析 OpenAI Device Code 响应中的 interval 字段
///
/// 服务端可能返回字符串或数字，需要兼容
fn parse_interval(value: Option<&serde_json::Value>) -> u64 {
    let raw = match value {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(5),
        Some(serde_json::Value::String(s)) => s.parse::<u64>().unwrap_or(5),
        _ => 5,
    };
    raw.max(1) + POLLING_SAFETY_MARGIN_SECS
}

/// 从 expires_in（秒）计算过期时间戳（毫秒）
fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let secs = expires_in.unwrap_or(3600);
    now_ms + secs * 1000
}

/// 解析 JWT 中的 claims
fn parse_jwt_claims(token: &str) -> Option<IdTokenClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let mut claims: IdTokenClaims = serde_json::from_slice(&decoded).ok()?;
    if let Some(openai_auth) = claims.openai_auth.as_mut() {
        openai_auth.chatgpt_plan_type = openai_auth
            .chatgpt_plan_type
            .take()
            .map(|plan_type| plan_type.trim().to_lowercase())
            .filter(|plan_type| !plan_type.is_empty());
    }
    Some(claims)
}

fn extract_subscription_metadata_from_tokens(
    tokens: &OAuthTokenResponse,
) -> Option<CodexSubscriptionMetadata> {
    let claims = parse_jwt_claims(tokens.id_token.as_deref()?)?;
    let openai_auth = claims.openai_auth.unwrap_or_default();
    let plan_renews_at = openai_auth.subscription_active_until();
    Some(CodexSubscriptionMetadata {
        plan_type: openai_auth.chatgpt_plan_type,
        plan_renews_at,
    })
}

/// 从 token 响应中提取 (account_id, email)
fn extract_identity_from_tokens(tokens: &OAuthTokenResponse) -> (Option<String>, Option<String>) {
    let mut account_id: Option<String> = None;
    let mut email: Option<String> = None;

    if let Some(id_token) = tokens.id_token.as_deref() {
        if let Some(claims) = parse_jwt_claims(id_token) {
            account_id = claims
                .chatgpt_account_id
                .clone()
                .or_else(|| {
                    claims
                        .openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
                .or_else(|| claims.organizations.first().and_then(|o| o.id.clone()));
            email = claims.email.clone();
        }
    }

    if account_id.is_none() {
        if let Some(claims) = parse_jwt_claims(&tokens.access_token) {
            account_id = claims
                .chatgpt_account_id
                .clone()
                .or_else(|| {
                    claims
                        .openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
                .or_else(|| claims.organizations.first().and_then(|o| o.id.clone()));
            if email.is_none() {
                email = claims.email.clone();
            }
        }
    }

    (account_id, email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    fn jwt_with_payload(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        format!("{header}.{payload}.")
    }

    fn parsed_openai_auth_claim(value: serde_json::Value) -> OpenAiAuthClaim {
        parse_jwt_claims(&jwt_with_payload(serde_json::json!({
            "https://api.openai.com/auth": value,
        })))
        .unwrap()
        .openai_auth
        .unwrap()
    }

    fn delayed_refresh_hook() -> (TestRefreshHook, Arc<Semaphore>, Arc<Semaphore>) {
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let hook_started = Arc::clone(&started);
        let hook_release = Arc::clone(&release);
        let hook: TestRefreshHook = Arc::new(move |_refresh_token| {
            let started = Arc::clone(&hook_started);
            let release = Arc::clone(&hook_release);
            Box::pin(async move {
                started.add_permits(1);
                let permit = release.acquire().await.unwrap();
                permit.forget();
                Ok(OAuthTokenResponse {
                    access_token: "refreshed-access-token".to_string(),
                    refresh_token: Some("rotated-refresh-token".to_string()),
                    id_token: None,
                    expires_in: Some(3600),
                })
            })
        });

        (hook, started, release)
    }

    fn delayed_first_persistence_snapshot_hook() -> (
        TestPersistenceHook,
        Arc<Semaphore>,
        Arc<Semaphore>,
        Arc<Semaphore>,
    ) {
        let first_started = Arc::new(Semaphore::new(0));
        let release_first = Arc::new(Semaphore::new(0));
        let later_started = Arc::new(Semaphore::new(0));
        let invocation_count = Arc::new(AtomicUsize::new(0));
        let hook_first_started = Arc::clone(&first_started);
        let hook_release_first = Arc::clone(&release_first);
        let hook_later_started = Arc::clone(&later_started);
        let hook_invocation_count = Arc::clone(&invocation_count);
        let hook: TestPersistenceHook = Arc::new(move || {
            let first_started = Arc::clone(&hook_first_started);
            let release_first = Arc::clone(&hook_release_first);
            let later_started = Arc::clone(&hook_later_started);
            let invocation = hook_invocation_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if invocation == 0 {
                    first_started.add_permits(1);
                    let permit = release_first.acquire().await.unwrap();
                    permit.forget();
                } else {
                    later_started.add_permits(1);
                }
            })
        });

        (hook, first_started, release_first, later_started)
    }

    async fn wait_for_refresh_request(started: &Semaphore) {
        let permit = tokio::time::timeout(std::time::Duration::from_secs(2), started.acquire())
            .await
            .expect("refresh should reach the test hook")
            .unwrap();
        permit.forget();
    }

    async fn wait_for_persistence_snapshot(started: &Semaphore) {
        let permit = tokio::time::timeout(std::time::Duration::from_secs(2), started.acquire())
            .await
            .expect("persistence should reach the snapshot hook")
            .unwrap();
        permit.forget();
    }

    async fn assert_account_and_token_absent(manager: &CodexOAuthManager, account_id: &str) {
        assert!(!manager.accounts.read().await.contains_key(account_id));
        assert!(!manager.access_tokens.read().await.contains_key(account_id));
    }

    async fn assert_account_absent_after_disk_reload(
        data_dir: &std::path::Path,
        account_id: &str,
        forbidden_refresh_tokens: &[&str],
    ) {
        let storage_path = data_dir.join("codex_oauth_auth.json");
        if let Ok(content) = fs::read_to_string(&storage_path) {
            assert!(!content.contains(account_id));
            for token in forbidden_refresh_tokens {
                assert!(!content.contains(token));
            }
        }

        let reloaded = CodexOAuthManager::new(data_dir.to_path_buf());
        let accounts = reloaded.accounts.read().await;
        assert!(!accounts.contains_key(account_id));
        assert!(accounts.values().all(|account| forbidden_refresh_tokens
            .iter()
            .all(|token| account.refresh_token != *token)));
        drop(accounts);
        assert!(!reloaded.access_tokens.read().await.contains_key(account_id));
        assert_ne!(
            reloaded.default_account_id().await.as_deref(),
            Some(account_id)
        );
    }

    #[tokio::test]
    async fn managed_quota_query_releases_outer_lock_while_token_lookup_waits() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(RwLock::new(CodexOAuthManager::new(
            temp.path().to_path_buf(),
        )));
        manager
            .read()
            .await
            .add_account_internal(
                "blocked-account".to_string(),
                "refresh-token".to_string(),
                None,
            )
            .await
            .unwrap();
        let access_tokens = {
            let manager = manager.read().await;
            Arc::clone(&manager.access_tokens)
        };

        // Force token lookup to remain pending without making a network request.
        let token_guard = access_tokens.write().await;
        let start_guard = manager.write().await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let query_manager = Arc::clone(&manager);
        let query_task = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            crate::providers::query_managed_codex_oauth_quota(
                &query_manager,
                Some("blocked-account"),
            )
            .await
        });

        // The query is now queued behind start_guard. Tokio's fair RwLock queue
        // guarantees it acquires the next read permit before this later writer.
        started_rx.await.unwrap();
        drop(start_guard);

        let outer_write = tokio::time::timeout(std::time::Duration::from_secs(1), manager.write())
            .await
            .expect("token lookup must not retain the outer manager read lock");
        drop(outer_write);

        query_task.abort();
        drop(token_guard);
        let _ = query_task.await;
    }

    #[tokio::test]
    async fn cached_token_fast_path_rejects_a_removed_account() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        manager
            .add_account_internal("acc-cached".to_string(), "refresh-token".to_string(), None)
            .await
            .unwrap();
        manager.access_tokens.write().await.insert(
            "acc-cached".to_string(),
            CachedAccessToken {
                token: "cached-access-token".to_string(),
                expires_at_ms: compute_expires_at_ms(Some(3600)),
            },
        );

        // Model the old removal window: the account disappeared before its
        // cached token was cleaned up. The fast path must still reject it.
        manager.accounts.write().await.remove("acc-cached");

        let result = manager.get_valid_token_for_account("acc-cached").await;
        assert!(matches!(
            result,
            Err(CodexOAuthError::AccountNotFound(id)) if id == "acc-cached"
        ));
    }

    #[tokio::test]
    async fn remove_account_during_refresh_does_not_resurrect_access_token() {
        let temp = tempfile::tempdir().unwrap();
        let (hook, started, release) = delayed_refresh_hook();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        *manager.refresh_hook.write().await = Some(hook);
        manager
            .add_account_internal(
                "acc-remove-race".to_string(),
                "refresh-token".to_string(),
                None,
            )
            .await
            .unwrap();
        let manager = Arc::new(manager);

        let refreshing_manager = Arc::clone(&manager);
        let refresh = tokio::spawn(async move {
            refreshing_manager
                .get_valid_token_for_account("acc-remove-race")
                .await
        });
        wait_for_refresh_request(&started).await;

        manager.remove_account("acc-remove-race").await.unwrap();
        release.add_permits(1);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), refresh)
            .await
            .expect("refresh task should finish")
            .unwrap();
        assert!(matches!(
            result,
            Err(CodexOAuthError::AccountNotFound(id)) if id == "acc-remove-race"
        ));
        assert_account_and_token_absent(&manager, "acc-remove-race").await;
    }

    #[tokio::test]
    async fn clear_auth_during_refresh_does_not_resurrect_access_token() {
        let temp = tempfile::tempdir().unwrap();
        let (hook, started, release) = delayed_refresh_hook();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        *manager.refresh_hook.write().await = Some(hook);
        manager
            .add_account_internal(
                "acc-clear-race".to_string(),
                "refresh-token".to_string(),
                None,
            )
            .await
            .unwrap();
        let manager = Arc::new(manager);

        let refreshing_manager = Arc::clone(&manager);
        let refresh = tokio::spawn(async move {
            refreshing_manager
                .get_valid_token_for_account("acc-clear-race")
                .await
        });
        wait_for_refresh_request(&started).await;

        manager.clear_auth().await.unwrap();
        release.add_permits(1);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), refresh)
            .await
            .expect("refresh task should finish")
            .unwrap();
        assert!(matches!(
            result,
            Err(CodexOAuthError::AccountNotFound(id)) if id == "acc-clear-race"
        ));
        assert_account_and_token_absent(&manager, "acc-clear-race").await;
    }

    #[tokio::test]
    async fn remove_account_waits_for_refresh_persistence_and_wins_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().to_path_buf();
        let (refresh_hook, refresh_started, release_refresh) = delayed_refresh_hook();
        let (persistence_hook, first_snapshot, release_first_snapshot, later_snapshot) =
            delayed_first_persistence_snapshot_hook();
        let manager = CodexOAuthManager::new(data_dir.clone());
        *manager.refresh_hook.write().await = Some(refresh_hook);
        manager
            .add_account_internal(
                "acc-remove-disk-race".to_string(),
                "original-refresh-token".to_string(),
                None,
            )
            .await
            .unwrap();
        *manager.persistence_snapshot_hook.write().await = Some(persistence_hook);
        let manager = Arc::new(manager);

        let refreshing_manager = Arc::clone(&manager);
        let refresh = tokio::spawn(async move {
            refreshing_manager
                .get_valid_token_for_account("acc-remove-disk-race")
                .await
        });
        wait_for_refresh_request(&refresh_started).await;
        release_refresh.add_permits(1);
        wait_for_persistence_snapshot(&first_snapshot).await;

        let removing_manager = Arc::clone(&manager);
        let (remove_started_tx, remove_started_rx) = tokio::sync::oneshot::channel();
        let remove = tokio::spawn(async move {
            remove_started_tx.send(()).unwrap();
            removing_manager
                .remove_account("acc-remove-disk-race")
                .await
        });
        remove_started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                later_snapshot.acquire()
            )
            .await
            .is_err(),
            "remove must not snapshot state while refresh persistence is paused"
        );

        release_first_snapshot.add_permits(1);
        wait_for_persistence_snapshot(&later_snapshot).await;
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), refresh)
                .await
                .expect("refresh task should finish")
                .unwrap()
                .unwrap(),
            "refreshed-access-token"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), remove)
            .await
            .expect("remove task should finish")
            .unwrap()
            .unwrap();

        assert_account_and_token_absent(&manager, "acc-remove-disk-race").await;
        assert_account_absent_after_disk_reload(
            &data_dir,
            "acc-remove-disk-race",
            &["original-refresh-token", "rotated-refresh-token"],
        )
        .await;
    }

    #[tokio::test]
    async fn clear_auth_waits_for_refresh_persistence_and_wins_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().to_path_buf();
        let (refresh_hook, refresh_started, release_refresh) = delayed_refresh_hook();
        let (persistence_hook, first_snapshot, release_first_snapshot, later_snapshot) =
            delayed_first_persistence_snapshot_hook();
        let manager = CodexOAuthManager::new(data_dir.clone());
        *manager.refresh_hook.write().await = Some(refresh_hook);
        manager
            .add_account_internal(
                "acc-clear-disk-race".to_string(),
                "original-refresh-token".to_string(),
                None,
            )
            .await
            .unwrap();
        *manager.persistence_snapshot_hook.write().await = Some(persistence_hook);
        let manager = Arc::new(manager);

        let refreshing_manager = Arc::clone(&manager);
        let refresh = tokio::spawn(async move {
            refreshing_manager
                .get_valid_token_for_account("acc-clear-disk-race")
                .await
        });
        wait_for_refresh_request(&refresh_started).await;
        release_refresh.add_permits(1);
        wait_for_persistence_snapshot(&first_snapshot).await;

        let clearing_manager = Arc::clone(&manager);
        let (clear_started_tx, clear_started_rx) = tokio::sync::oneshot::channel();
        let clear = tokio::spawn(async move {
            clear_started_tx.send(()).unwrap();
            clearing_manager.clear_auth().await
        });
        clear_started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                later_snapshot.acquire()
            )
            .await
            .is_err(),
            "clear must not snapshot state while refresh persistence is paused"
        );

        release_first_snapshot.add_permits(1);
        wait_for_persistence_snapshot(&later_snapshot).await;
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), refresh)
                .await
                .expect("refresh task should finish")
                .unwrap()
                .unwrap(),
            "refreshed-access-token"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), clear)
            .await
            .expect("clear task should finish")
            .unwrap()
            .unwrap();

        assert_account_and_token_absent(&manager, "acc-clear-disk-race").await;
        assert_account_absent_after_disk_reload(
            &data_dir,
            "acc-clear-disk-race",
            &["original-refresh-token", "rotated-refresh-token"],
        )
        .await;
    }

    #[test]
    fn codex_oauth_user_agent_preserves_legacy_wire_bytes() {
        assert_eq!(CODEX_USER_AGENT, "cc-switch-codex-oauth");
    }

    #[test]
    fn token_refresh_log_message_is_identity_free() {
        assert_eq!(
            TOKEN_REFRESH_LOG_MESSAGE,
            "[CodexOAuth] access_token refresh required"
        );
        assert!(!TOKEN_REFRESH_LOG_MESSAGE.contains("account_id"));
    }

    #[test]
    fn test_parse_interval_number() {
        let v = serde_json::Value::Number(serde_json::Number::from(5));
        assert_eq!(parse_interval(Some(&v)), 5 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_string() {
        let v = serde_json::Value::String("10".to_string());
        assert_eq!(parse_interval(Some(&v)), 10 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_default() {
        assert_eq!(parse_interval(None), 5 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_min() {
        let v = serde_json::Value::Number(serde_json::Number::from(0));
        // 0 应被提升到 1
        assert_eq!(parse_interval(Some(&v)), 1 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_compute_expires_at_ms() {
        let result = compute_expires_at_ms(Some(3600));
        let now = chrono::Utc::now().timestamp_millis();
        // 应在未来约 3600 秒处（允许少量误差）
        assert!(result > now + 3500 * 1000);
        assert!(result < now + 3700 * 1000);
    }

    #[test]
    fn test_compute_expires_at_ms_default() {
        let result = compute_expires_at_ms(None);
        let now = chrono::Utc::now().timestamp_millis();
        assert!(result > now);
    }

    #[test]
    fn test_cached_token_expiring_soon() {
        let now = chrono::Utc::now().timestamp_millis();
        // 30 秒后过期 - 在缓冲期内
        let expiring = CachedAccessToken {
            token: "t".to_string(),
            expires_at_ms: now + 30_000,
        };
        assert!(expiring.is_expiring_soon());

        // 1 小时后过期 - 不在缓冲期内
        let valid = CachedAccessToken {
            token: "t".to_string(),
            expires_at_ms: now + 3_600_000,
        };
        assert!(!valid.is_expiring_soon());
    }

    #[test]
    fn test_parse_jwt_claims_invalid() {
        assert!(parse_jwt_claims("not-a-jwt").is_none());
        assert!(parse_jwt_claims("only.two").is_none());
    }

    #[test]
    fn test_parse_jwt_claims_valid() {
        // Header: {"alg":"none"}
        // Payload: {"chatgpt_account_id":"acc-123","email":"test@example.com"}
        // Signature: empty
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD
            .encode(b"{\"chatgpt_account_id\":\"acc-123\",\"email\":\"test@example.com\"}");
        let jwt = format!("{header}.{payload}.");
        let claims = parse_jwt_claims(&jwt).unwrap();
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("acc-123"));
        assert_eq!(claims.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn openai_auth_claim_normalizes_chatgpt_plan_type() {
        let populated = parsed_openai_auth_claim(serde_json::json!({
            "chatgpt_account_id": "acc-123",
            "chatgpt_plan_type": "  Pro  ",
        }));
        assert_eq!(populated.chatgpt_plan_type.as_deref(), Some("pro"));

        let empty = parsed_openai_auth_claim(serde_json::json!({
            "chatgpt_plan_type": " \t ",
        }));
        assert_eq!(empty.chatgpt_plan_type, None);
    }

    #[test]
    fn openai_auth_claim_parses_subscription_active_until_formats() {
        let number = parsed_openai_auth_claim(serde_json::json!({
            "chatgpt_subscription_active_until": 1_789_876_543,
        }));
        assert_eq!(number.subscription_active_until(), Some(1_789_876_543));

        let numeric_string = parsed_openai_auth_claim(serde_json::json!({
            "chatgpt_subscription_active_until": "1789876543",
        }));
        assert_eq!(
            numeric_string.subscription_active_until(),
            Some(1_789_876_543)
        );

        for value in [
            serde_json::json!({ "chatgpt_subscription_active_until": null }),
            serde_json::json!({}),
            serde_json::json!({ "chatgpt_subscription_active_until": "later" }),
            serde_json::json!({ "chatgpt_subscription_active_until": true }),
        ] {
            assert_eq!(
                parsed_openai_auth_claim(value).subscription_active_until(),
                None
            );
        }
    }

    #[test]
    fn test_parse_jwt_claims_organizations_fallback() {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD.encode(b"{\"organizations\":[{\"id\":\"org-456\"}]}");
        let jwt = format!("{header}.{payload}.");
        let claims = parse_jwt_claims(&jwt).unwrap();
        assert_eq!(
            claims
                .organizations
                .first()
                .and_then(|o| o.id.clone())
                .as_deref(),
            Some("org-456")
        );
    }

    #[tokio::test]
    async fn test_manager_initial_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        assert!(!manager.is_authenticated().await);
        assert!(manager.list_accounts().await.is_empty());
    }

    #[tokio::test]
    async fn test_manager_save_and_load() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_path_buf();

        // Manually inject an account through internal methods
        {
            let manager = CodexOAuthManager::new(path.clone());
            manager
                .add_account_internal(
                    "acc-123".to_string(),
                    "rt-secret".to_string(),
                    Some("user@example.com".to_string()),
                )
                .await
                .unwrap();
        }

        // New manager should load from disk
        let manager2 = CodexOAuthManager::new(path);
        let accounts = manager2.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "acc-123");
    }

    #[tokio::test]
    async fn test_remove_account() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .add_account_internal(
                "acc-123".to_string(),
                "rt".to_string(),
                Some("a@example.com".to_string()),
            )
            .await
            .unwrap();
        manager
            .add_account_internal(
                "acc-456".to_string(),
                "rt2".to_string(),
                Some("b@example.com".to_string()),
            )
            .await
            .unwrap();

        manager.remove_account("acc-123").await.unwrap();
        let accounts = manager.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "acc-456");
    }
}
