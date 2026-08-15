//! PackyCode 账户用量(余额 / 已用 / 参考额度)模块(D2)。
//!
//! PackyCode 是 NewAPI 搭建的中转,余额与花费只有 NewAPI 管理接口能读,
//! 模型 key 读不到 —— 凭据是「系统访问令牌 + 用户 ID」,与供应商 API key
//! 是两套互不通用。本模块只负责读取并暴露三个数,不做 UI、不做今日花费上条。
//!
//! 存储采用方案 A:令牌进钥匙串(`packycode-usage-token`),user_id / base_url /
//! 快照进 settings 三个键。不建表、不动 SCHEMA_VERSION、无迁移 —— 单账号的
//! 一个快照用不着一张表,删掉这个功能就是删一行 settings。
//!
//! 分层照 official_pricing.rs:网络层(可注入 client)、换算层(纯函数)、
//! 存储层、调度器(15 分钟 + 失败退避)、对外视图。上游前面有 Cloudflare
//! 防护,任何非 JSON / 非 2xx 响应都按错误码处理,绝不 panic、不从 HTML
//! 里抠字段;真实验收需在用户本机用真实令牌跑。

use crate::error::AppError;
use crate::secrets::CredentialStore;
use crate::store::{lock_conn, Database};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

// ── 常量 ────────────────────────────────────────────────────

/// 系统访问令牌的钥匙串 slot(单账号,固定 slot,不需要 staging/journal)。
pub const TOKEN_SLOT: &str = "packycode-usage-token";
/// settings 键:用户 ID(非机密)。
pub const SETTING_USER_ID: &str = "packycode_usage_user_id";
/// settings 键:base_url(默认 `https://cf.api.fan`,端点路径前缀可配)。
pub const SETTING_BASE_URL: &str = "packycode_usage_base_url";
/// settings 键:账户用量快照 JSON。
pub const SETTING_SNAPSHOT: &str = "packycode_account_usage";

pub const DEFAULT_BASE_URL: &str = "https://cf.api.fan";
/// NewAPI 默认换算口径:500000 单位 = 1 USD。运营方可改,绝不能写死。
pub const DEFAULT_QUOTA_PER_UNIT: i64 = 500_000;

/// 单请求超时。防护页/慢上游都必须尽快失败,不能让后台线程挂 8 秒以上。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// 与 key usage 调度一致的周期。
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// 失败退避阶梯,照 official_pricing 的形状。
pub const FAILURE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(600),
    Duration::from_secs(1_800),
];

/// 手动刷新防连点:60 秒内重复调用复用上次快照。
const MANUAL_REFRESH_MIN_INTERVAL_SECS: i64 = 60;

/// 本项目自己的 UA 字符串(NewAPI 对来源无要求,统一带上)。
const PACKYCODE_USER_AGENT: &str = crate::product_identity::APP_SLUG;

/// 上游 message 可能含运营方文案,记日志/进错误串前截断,防超长文本刷屏。
const UPSTREAM_MESSAGE_MAX_CHARS: usize = 200;

// ── 错误码 ──────────────────────────────────────────────────

/// 网络层四类错误码。Display 里带机器可读前缀,便于前端/测试区分。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackyCodeFetchError {
    /// 401/403:上游拒绝凭据。
    #[error("authentication_failed: 上游拒绝凭据 (HTTP {status})")]
    AuthenticationFailed { status: u16 },
    /// 网络不可达/超时/读体中断。
    #[error("connection_failed: {detail}")]
    ConnectionFailed { detail: String },
    /// 非 JSON / 缺字段 / 字段类型或取值非法(含 Cloudflare HTML 防护页)。
    #[error("invalid_response: {detail}")]
    InvalidResponse { detail: String },
    /// 信封 `success: false`:上游明确拒绝,带上(已截断的)message。
    #[error("upstream_rejected: {message}")]
    UpstreamRejected { message: String },
}

impl PackyCodeFetchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AuthenticationFailed { .. } => "authentication_failed",
            Self::ConnectionFailed { .. } => "connection_failed",
            Self::InvalidResponse { .. } => "invalid_response",
            Self::UpstreamRejected { .. } => "upstream_rejected",
        }
    }
}

impl From<PackyCodeFetchError> for AppError {
    fn from(error: PackyCodeFetchError) -> Self {
        AppError::Message(error.to_string())
    }
}

// ── 对外结构 ────────────────────────────────────────────────

/// `/api/user/self` 的 data 里本模块消费的两个字段。
/// 只认任务书 §1.2 的字段;quota/used_quota 缺失、负数、非整数一律
/// invalid_response,绝不静默当 0(快照只存金额,其余字段不消费)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfPayload {
    /// 剩余额度(整数单位)。
    pub quota: i64,
    /// 累计已用(整数单位)。
    pub used_quota: i64,
}

/// settings 里 `packycode_account_usage` 的落盘形态(方案 A 快照)。
/// 金额一律十进制字符串,不存浮点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    /// 剩余 USD = quota / quota_per_unit。
    pub balance_usd: String,
    /// 累计已用 USD = used_quota / quota_per_unit。
    pub used_usd: String,
    /// 参考额度(累计充值)USD = (quota + used_quota) / quota_per_unit。
    pub reference_usd: String,
    /// 展示换算口径,便于诊断。
    pub quota_per_unit: String,
    pub fetched_at: i64,
}

/// 对外视图:只暴露视图,不暴露令牌明文;无快照时全 None,绝不编造 0。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackyCodeAccountUsageView {
    pub balance_usd: Option<String>,
    pub used_usd: Option<String>,
    pub reference_usd: Option<String>,
    pub quota_per_unit: Option<String>,
    pub fetched_at: Option<i64>,
    pub has_credentials: bool,
}

// ── 换算层(纯函数)─────────────────────────────────────────

/// quota 整数单位 → USD 十进制字符串。
///
/// 用 `rust_decimal::Decimal` 精确除法,输出 normalize 过的字符串;
/// `quota_per_unit <= 0` 按默认 500000 计。换算口径来自上游,这里只消费,
/// 不写死 500000 为唯一口径。
pub fn quota_units_to_usd(units: i64, quota_per_unit: i64) -> String {
    let per_unit = if quota_per_unit > 0 {
        quota_per_unit
    } else {
        DEFAULT_QUOTA_PER_UNIT
    };
    let usd = Decimal::from(units) / Decimal::from(per_unit);
    usd.normalize().to_string()
}

// ── 网络层(可注入 client,便于单测)─────────────────────────

/// 拼出挂在 base 下的管理端点。base 带 `/v1` 前缀时路径自然带上,
/// 前缀不做任何写死假设(任务书 §1.4)。
fn endpoint(base_url: &str, path: &str) -> Result<String, PackyCodeFetchError> {
    let raw = format!("{}{}", base_url.trim().trim_end_matches('/'), path);
    let parsed = url::Url::parse(&raw).map_err(|error| PackyCodeFetchError::InvalidResponse {
        detail: format!("base_url 无法解析: {error}"),
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        _ => Err(PackyCodeFetchError::InvalidResponse {
            detail: "base_url 不是 http(s) 地址".to_string(),
        }),
    }
}

/// 读 `GET {base}/api/status` 的 `quota_per_unit`。
///
/// 404 / 非 JSON / 超时 / 字段缺失或非法 → `Ok(None)`(调用方回退 500000),
/// **不视为硬失败** —— 防护页经常 403/404,不能因此判整个刷新失败。
pub async fn fetch_status(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Option<String>, PackyCodeFetchError> {
    let url = endpoint(base_url, "/api/status")?;
    let response = match client
        .get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", PACKYCODE_USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let document: Value = match serde_json::from_slice(&bytes) {
        Ok(document) => document,
        Err(_) => return Ok(None),
    };
    Ok(parse_quota_per_unit(&document))
}

/// 只认正整数;缺失 / 0 / 负数 / 非整数 → None(回退默认口径)。
fn parse_quota_per_unit(document: &Value) -> Option<String> {
    let number = document.get("quota_per_unit")?.as_i64()?;
    (number > 0).then(|| number.to_string())
}

/// 读 `GET {base}/api/user/self`(两头发齐:系统访问令牌 + 用户 ID)。
///
/// 分类顺序有意为之:401/403 先于 body 解析 —— Cloudflare 挑战页常以 403
/// 返回 HTML,这种响应就是「凭据被拦」,而不是「响应看不懂」;200 的 HTML
/// 才是 invalid_response。任何非 JSON / 缺字段都绝不 panic、不抠字段。
pub async fn fetch_self(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    user_id: &str,
) -> Result<SelfPayload, PackyCodeFetchError> {
    let url = endpoint(base_url, "/api/user/self")?;
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("New-Api-User", user_id)
        .header("User-Agent", PACKYCODE_USER_AGENT)
        .header("Accept", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| PackyCodeFetchError::ConnectionFailed {
            detail: error.to_string(),
        })?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(PackyCodeFetchError::AuthenticationFailed {
            status: status.as_u16(),
        });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| PackyCodeFetchError::ConnectionFailed {
            detail: error.to_string(),
        })?;
    if !status.is_success() {
        return Err(PackyCodeFetchError::InvalidResponse {
            detail: format!("HTTP {status}"),
        });
    }
    parse_self_document(&bytes)
}

/// 解析 `{ success, message, data }` 信封。success 缺失/非布尔、data 缺失
/// 或字段非法 → invalid_response;success:false → upstream_rejected。
fn parse_self_document(bytes: &[u8]) -> Result<SelfPayload, PackyCodeFetchError> {
    let document: Value =
        serde_json::from_slice(bytes).map_err(|error| PackyCodeFetchError::InvalidResponse {
            detail: format!("非 JSON: {error}"),
        })?;
    match document.get("success").and_then(Value::as_bool) {
        Some(true) => {}
        Some(false) => {
            let message = document
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(PackyCodeFetchError::UpstreamRejected {
                message: truncate_upstream_message(message),
            });
        }
        None => {
            return Err(PackyCodeFetchError::InvalidResponse {
                detail: "缺少 success 字段".to_string(),
            });
        }
    }
    let data = document
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| PackyCodeFetchError::InvalidResponse {
            detail: "缺少 data 对象".to_string(),
        })?;
    Ok(SelfPayload {
        quota: parse_non_negative_i64(data, "quota")?,
        used_quota: parse_non_negative_i64(data, "used_quota")?,
    })
}

fn parse_non_negative_i64(
    data: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<i64, PackyCodeFetchError> {
    match data.get(field).and_then(Value::as_i64) {
        Some(value) if value >= 0 => Ok(value),
        _ => Err(PackyCodeFetchError::InvalidResponse {
            detail: format!("字段 {field} 缺失、非整数或为负数"),
        }),
    }
}

fn truncate_upstream_message(message: &str) -> String {
    message.chars().take(UPSTREAM_MESSAGE_MAX_CHARS).collect()
}

// ── 存储层(方案 A:钥匙串 + settings 三个键)─────────────────

struct Credentials {
    token: String,
    user_id: String,
    base_url: String,
}

/// 读凭据。令牌缺失/为空,或 user_id 缺失 → None(未配置)。
/// 钥匙串读取失败向上抛:调用方(视图)按未配置降级,写路径直接失败。
fn read_credentials(
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<Option<Credentials>, AppError> {
    let Some(bytes) = credential_store
        .get(TOKEN_SLOT)
        .map_err(|error| AppError::Message(format!("credential_store_failed: {error}")))?
    else {
        return Ok(None);
    };
    let token = String::from_utf8(bytes)
        .map_err(|_| AppError::Message("packycode 用量令牌损坏".to_string()))?;
    let user_id = match db.get_setting(SETTING_USER_ID)? {
        Some(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let base_url = match db.get_setting(SETTING_BASE_URL)? {
        Some(value) if !value.trim().is_empty() => value,
        _ => DEFAULT_BASE_URL.to_string(),
    };
    Ok((!token.is_empty()).then_some(Credentials {
        token,
        user_id,
        base_url,
    }))
}

/// 令牌 + 用户 ID 都在才算配好。读失败一律按未配置(视图不报错)。
fn has_credentials(db: &Database, credential_store: &dyn CredentialStore) -> bool {
    match credential_store.get(TOKEN_SLOT) {
        Ok(Some(bytes)) => {
            !bytes.is_empty()
                && db
                    .get_setting(SETTING_USER_ID)
                    .ok()
                    .flatten()
                    .is_some_and(|value| !value.trim().is_empty())
        }
        Ok(None) => false,
        Err(error) => {
            log::warn!("packycode 用量令牌读取失败,视为未配置: {error}");
            false
        }
    }
}

/// 读快照。读侧宽容:JSON 损坏按无快照处理,不挡视图。
fn read_snapshot(db: &Database) -> Result<Option<AccountSnapshot>, AppError> {
    match db.get_setting(SETTING_SNAPSHOT)? {
        None => Ok(None),
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(error) => {
                log::warn!("packycode 用量快照损坏,按无快照处理: {error}");
                Ok(None)
            }
        },
    }
}

/// 成功才落盘;失败路径绝不碰旧快照。
fn write_snapshot(db: &Database, snapshot: &AccountSnapshot) -> Result<(), AppError> {
    db.set_setting(
        SETTING_SNAPSHOT,
        &serde_json::to_string(snapshot)
            .map_err(|error| AppError::JsonSerialize { source: error })?,
    )
}

fn delete_setting(db: &Database, key: &str) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])
        .map_err(AppError::from)?;
    Ok(())
}

// ── 编排:刷新 / 配置 / 视图 ────────────────────────────────

/// 抓一次并落快照(调度器与手动刷新共用)。只有全部成功才写 settings;
/// 成功后会通知面板失效,让前端重取视图。
pub async fn refresh_account_usage(
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<AccountSnapshot, AppError> {
    refresh_account_usage_with_client(&crate::http_client::get(), db, credential_store).await
}

/// 与 refresh_account_usage 相同的编排,client 可注入供单测指向本地假服务。
async fn refresh_account_usage_with_client(
    client: &reqwest::Client,
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<AccountSnapshot, AppError> {
    let Some(credentials) = read_credentials(db, credential_store)? else {
        return Err(AppError::Message("no_credentials".to_string()));
    };

    // /api/status 读不到口径就回退 500000,不视为硬失败。
    let quota_per_unit = fetch_status(client, &credentials.base_url)
        .await?
        .unwrap_or_else(|| DEFAULT_QUOTA_PER_UNIT.to_string());
    let per_unit = quota_per_unit.parse::<i64>().unwrap_or(0);

    let payload = fetch_self(
        client,
        &credentials.base_url,
        &credentials.token,
        &credentials.user_id,
    )
    .await?;

    let snapshot = AccountSnapshot {
        balance_usd: quota_units_to_usd(payload.quota, per_unit),
        used_usd: quota_units_to_usd(payload.used_quota, per_unit),
        reference_usd: quota_units_to_usd(
            payload.quota.saturating_add(payload.used_quota),
            per_unit,
        ),
        quota_per_unit,
        fetched_at: unix_timestamp()?,
    };
    write_snapshot(db, &snapshot)?;
    crate::usage_events::notify_dashboard_invalidated();
    log::info!(
        "packycode 账户用量已刷新: balance_usd={}, used_usd={}, reference_usd={}",
        snapshot.balance_usd,
        snapshot.used_usd,
        snapshot.reference_usd
    );
    Ok(snapshot)
}

/// 手动刷新入口:60 秒内重复调用复用上次快照(防连点),不重复请求。
pub async fn manual_refresh(
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<PackyCodeAccountUsageView, AppError> {
    manual_refresh_with_client(&crate::http_client::get(), db, credential_store).await
}

/// 与 manual_refresh 相同的编排,client 可注入供单测指向本地假服务。
async fn manual_refresh_with_client(
    client: &reqwest::Client,
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<PackyCodeAccountUsageView, AppError> {
    let now = unix_timestamp()?;
    if let Some(snapshot) = read_snapshot(db)? {
        if now.saturating_sub(snapshot.fetched_at) < MANUAL_REFRESH_MIN_INTERVAL_SECS {
            log::debug!("packycode 用量刷新过于频繁,复用上次快照");
            return account_usage_view(db, credential_store);
        }
    }
    refresh_account_usage_with_client(client, db, credential_store).await?;
    account_usage_view(db, credential_store)
}

/// set:trim 后令牌进钥匙串、user_id/base_url 进 settings;
/// 空令牌/空 user_id 拒绝;写入前先 fetch_self 验证一次,成功才算配好,
/// 失败不落盘。日志与错误串绝不含令牌明文。
pub async fn set_credentials(
    db: &Database,
    credential_store: &dyn CredentialStore,
    access_token: String,
    user_id: String,
    base_url: Option<String>,
) -> Result<(), AppError> {
    set_credentials_with_client(
        &crate::http_client::get(),
        db,
        credential_store,
        access_token,
        user_id,
        base_url,
    )
    .await
}

async fn set_credentials_with_client(
    client: &reqwest::Client,
    db: &Database,
    credential_store: &dyn CredentialStore,
    access_token: String,
    user_id: String,
    base_url: Option<String>,
) -> Result<(), AppError> {
    let token = access_token.trim().to_string();
    let user_id = user_id.trim().to_string();
    if token.is_empty() || user_id.is_empty() {
        return Err(AppError::InvalidInput(
            "系统访问令牌与用户 ID 都不能为空".to_string(),
        ));
    }
    let base_url = base_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    // 先验证再落盘:fetch_self 成功才算配好,失败不写任何东西。
    // 令牌只进请求头,错误串里只有错误码与状态,绝无明文。
    if let Err(error) = fetch_self(client, &base_url, &token, &user_id).await {
        log::warn!(
            "packycode 用量凭据验证失败(code={}),已放弃写入",
            error.code()
        );
        return Err(error.into());
    }

    credential_store
        .put(TOKEN_SLOT, token.as_bytes())
        .map_err(|error| AppError::Message(format!("credential_store_failed: {error}")))?;
    db.set_setting(SETTING_USER_ID, &user_id)?;
    db.set_setting(SETTING_BASE_URL, &base_url)?;
    log::info!("packycode 用量凭据已保存(令牌内容不记日志)");
    Ok(())
}

/// clear:删钥匙串条目 + 清三个 settings 键。钥匙串删除失败时不继续,
/// 避免「界面显示已清、令牌还在钥匙串」的半清状态。
pub fn clear_credentials(
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<(), AppError> {
    credential_store
        .delete(TOKEN_SLOT)
        .map_err(|error| AppError::Message(format!("credential_store_failed: {error}")))?;
    delete_setting(db, SETTING_USER_ID)?;
    delete_setting(db, SETTING_BASE_URL)?;
    delete_setting(db, SETTING_SNAPSHOT)?;
    log::info!("packycode 用量凭据已清除");
    Ok(())
}

/// 只读视图,不发请求。无快照时金额全 None,绝不编造 0。
pub fn account_usage_view(
    db: &Database,
    credential_store: &dyn CredentialStore,
) -> Result<PackyCodeAccountUsageView, AppError> {
    let snapshot = read_snapshot(db)?;
    Ok(PackyCodeAccountUsageView {
        balance_usd: snapshot
            .as_ref()
            .map(|snapshot| snapshot.balance_usd.clone()),
        used_usd: snapshot.as_ref().map(|snapshot| snapshot.used_usd.clone()),
        reference_usd: snapshot
            .as_ref()
            .map(|snapshot| snapshot.reference_usd.clone()),
        quota_per_unit: snapshot
            .as_ref()
            .map(|snapshot| snapshot.quota_per_unit.clone()),
        fetched_at: snapshot.map(|snapshot| snapshot.fetched_at),
        has_credentials: has_credentials(db, credential_store),
    })
}

// ── 调度器(照 official_pricing.rs 的 start_scheduler 模板)──

type RefreshFuture<'a> = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + 'a>>;

/// 调度器依赖注入缝:生产实现落在编排函数上,测试用假实现驱动。
trait PackyCodeUsageDependencies: Send + Sync {
    fn refresh(&self) -> RefreshFuture<'_>;
    fn is_lightweight_mode(&self) -> bool;
    fn has_credentials(&self) -> bool;
}

struct ProductionDependencies {
    db: Arc<Database>,
    credential_store: Arc<dyn CredentialStore>,
}

impl PackyCodeUsageDependencies for ProductionDependencies {
    fn refresh(&self) -> RefreshFuture<'_> {
        let db = self.db.clone();
        let credential_store = self.credential_store.clone();
        Box::pin(async move {
            refresh_account_usage(&db, credential_store.as_ref())
                .await
                .map(|_| ())
        })
    }

    fn is_lightweight_mode(&self) -> bool {
        crate::lightweight::is_lightweight_mode()
    }

    fn has_credentials(&self) -> bool {
        has_credentials(&self.db, self.credential_store.as_ref())
    }
}

pub struct PackyCodeUsageSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl PackyCodeUsageSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for PackyCodeUsageSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn start_scheduler(
    db: Arc<Database>,
    credential_store: Arc<dyn CredentialStore>,
) -> PackyCodeUsageSchedulerHandle {
    start_scheduler_with_dependencies(
        Arc::new(ProductionDependencies {
            db,
            credential_store,
        }),
        REFRESH_INTERVAL,
        &FAILURE_RETRY_DELAYS,
    )
}

/// 周期与退避阶梯可注入,测试不必真等 15 分钟/30 秒。
fn start_scheduler_with_dependencies(
    dependencies: Arc<dyn PackyCodeUsageDependencies>,
    refresh_interval: Duration,
    failure_retry_delays: &'static [Duration],
) -> PackyCodeUsageSchedulerHandle {
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let task = tauri::async_runtime::spawn(async move {
        if !refresh_with_backoff_or_cancel(&*dependencies, failure_retry_delays, &mut cancel_rx)
            .await
        {
            return;
        }
        loop {
            if !wait_or_cancel(refresh_interval, &mut cancel_rx).await {
                return;
            }
            if !refresh_with_backoff_or_cancel(&*dependencies, failure_retry_delays, &mut cancel_rx)
                .await
            {
                return;
            }
        }
    });
    PackyCodeUsageSchedulerHandle {
        cancel_tx,
        task: Some(task),
    }
}

/// 一轮刷新;cancelled 时立刻退出。刷新与取消赛跑,取消赢则丢弃
/// 还在背睡的重试 future,保证「cancel 即时退出」。
async fn refresh_with_backoff_or_cancel(
    dependencies: &dyn PackyCodeUsageDependencies,
    failure_retry_delays: &[Duration],
    cancel_rx: &mut watch::Receiver<bool>,
) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = refresh_with_backoff(dependencies, failure_retry_delays) => true,
    }
}

/// 一轮「刷新 + 失败退避重试」:轻量模式不跑、未配置令牌空转
/// (不报错、不发请求),失败按阶梯等待后重试,成功即返回。
async fn refresh_with_backoff(
    dependencies: &dyn PackyCodeUsageDependencies,
    failure_retry_delays: &[Duration],
) {
    if dependencies.is_lightweight_mode() || !dependencies.has_credentials() {
        return;
    }
    let mut failure_count = 0usize;
    loop {
        match dependencies.refresh().await {
            Ok(()) => return,
            Err(error) => {
                let delay = failure_retry_delay(failure_count, failure_retry_delays);
                failure_count = failure_count.saturating_add(1);
                log::warn!(
                    "packycode 账户用量刷新失败(连续第 {failure_count} 次),{delay:?} 后重试: {error}"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// 第 n 次失败(从 0 数)对应的退避时长,超长按最后一档。
fn failure_retry_delay(consecutive_failures: usize, delays: &[Duration]) -> Duration {
    let index = consecutive_failures.min(delays.len().saturating_sub(1));
    delays[index]
}

async fn wait_or_cancel(duration: Duration, cancel_rx: &mut watch::Receiver<bool>) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = tokio::time::sleep(duration) => true,
    }
}

fn unix_timestamp() -> Result<i64, AppError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AppError::Message(format!("system clock before unix epoch: {error}")))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| AppError::Message("system timestamp exceeds i64".to_string()))
}

// ── 测试 ────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::CredentialStoreError;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tower::ServiceExt;

    #[derive(Default)]
    struct MemoryCredentialStore {
        items: Mutex<HashMap<String, Vec<u8>>>,
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
            self.items.lock().unwrap().remove(slot);
            Ok(())
        }
    }

    fn memory_db() -> Database {
        Database::memory().unwrap()
    }

    /// 测试用 client:关掉代理探测,避免开发者本机代理把 127.0.0.1 的
    /// 假服务流量劫走;生产代码用 crate::http_client::get(),测试注入它。
    fn test_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// 起一个本地假 NewAPI 服务,返回 base_url。
    async fn spawn_server(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{address}")
    }

    /// 直接调 handler 测解析(不必起网):axum Router 的 oneshot 调用。
    async fn call_handler(router: Router, request: Request<Body>) -> Response {
        router.oneshot(request).await.unwrap()
    }

    // ── §6.1:正常解析 + 非默认 quota_per_unit 换算 ─────────

    #[test]
    fn quota_units_to_usd_uses_the_upstream_ratio_not_a_hardcoded_500000() {
        // 非默认口径(1_000_000 = 1 USD)下换算公式仍成立 —— 证明没写死 500000。
        assert_eq!(quota_units_to_usd(1_234_567, 1_000_000), "1.234567");
        assert_eq!(quota_units_to_usd(234_567, 1_000_000), "0.234567");
        assert_eq!(quota_units_to_usd(1_469_134, 1_000_000), "1.469134");
        // 默认口径 + normalize(去掉末尾零)。
        assert_eq!(quota_units_to_usd(30_700_000, 500_000), "61.4");
        assert_eq!(quota_units_to_usd(0, 500_000), "0");
    }

    #[test]
    fn quota_units_to_usd_falls_back_to_500000_when_ratio_is_missing_or_zero() {
        assert_eq!(quota_units_to_usd(500_000, 0), "1");
        assert_eq!(quota_units_to_usd(500_000, -7), "1");
    }

    #[tokio::test]
    async fn refresh_parses_quota_and_used_quota_with_upstream_ratio() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store.put(TOKEN_SLOT, b"test-token".as_slice()).unwrap();
        db.set_setting(SETTING_USER_ID, "user-1").unwrap();
        let base = spawn_server(
            Router::new()
                .route(
                    "/api/status",
                    get(|| async { axum::Json(json!({ "quota_per_unit": 1_000_000 })) }),
                )
                .route(
                    "/api/user/self",
                    get(|| async {
                        axum::Json(json!({
                            "success": true,
                            "message": "",
                            "data": { "quota": 1_234_567, "used_quota": 234_567 }
                        }))
                    }),
                ),
        )
        .await;
        db.set_setting(SETTING_BASE_URL, &base).unwrap();

        let snapshot = refresh_account_usage_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();

        assert_eq!(snapshot.balance_usd, "1.234567");
        assert_eq!(snapshot.used_usd, "0.234567");
        assert_eq!(snapshot.reference_usd, "1.469134");
        assert_eq!(snapshot.quota_per_unit, "1000000");
        // 落盘的快照与返回一致。
        assert_eq!(read_snapshot(&db).unwrap(), Some(snapshot));
    }

    // ── §6.2:quota_per_unit 缺失/为 0/404 → 回退 500000 ─────

    #[test]
    fn parse_quota_per_unit_accepts_only_positive_integers() {
        assert_eq!(
            parse_quota_per_unit(&json!({ "quota_per_unit": 500_000 })),
            Some("500000".to_string())
        );
        assert_eq!(parse_quota_per_unit(&json!({})), None);
        assert_eq!(parse_quota_per_unit(&json!({ "quota_per_unit": 0 })), None);
        assert_eq!(parse_quota_per_unit(&json!({ "quota_per_unit": -1 })), None);
        assert_eq!(
            parse_quota_per_unit(&json!({ "quota_per_unit": "500000" })),
            None
        );
        assert_eq!(
            parse_quota_per_unit(&json!({ "quota_per_unit": 1.5 })),
            None
        );
    }

    #[tokio::test]
    async fn fetch_status_404_or_bad_json_falls_back_to_none() {
        let base = spawn_server(Router::new().route(
            "/api/status",
            get(|| async { (StatusCode::NOT_FOUND, "not found") }),
        ))
        .await;
        assert_eq!(fetch_status(&test_client(), &base).await.unwrap(), None);
    }

    #[tokio::test]
    async fn refresh_falls_back_to_500000_when_status_missing_the_ratio() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store.put(TOKEN_SLOT, b"test-token".as_slice()).unwrap();
        db.set_setting(SETTING_USER_ID, "user-1").unwrap();
        let base = spawn_server(
            Router::new()
                .route(
                    "/api/status",
                    get(|| async { axum::Json(json!({ "version": "v1.0.0" })) }),
                )
                .route(
                    "/api/user/self",
                    get(|| async {
                        axum::Json(json!({
                            "success": true,
                            "message": "",
                            "data": { "quota": 1_000_000, "used_quota": 0 }
                        }))
                    }),
                ),
        )
        .await;
        db.set_setting(SETTING_BASE_URL, &base).unwrap();

        let snapshot = refresh_account_usage_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();

        assert_eq!(snapshot.quota_per_unit, "500000");
        assert_eq!(snapshot.balance_usd, "2");
    }

    // ── §6.3:success:false → upstream_rejected,不写快照;HTML 不 panic ──

    #[test]
    fn success_false_is_upstream_rejected_with_message_truncated_to_200_chars() {
        let long_message = "拒".repeat(250);
        let document = serde_json::to_vec(&json!({
            "success": false,
            "message": long_message,
            "data": null
        }))
        .unwrap();

        let error = parse_self_document(&document).unwrap_err();

        assert_eq!(error.code(), "upstream_rejected");
        let PackyCodeFetchError::UpstreamRejected { message } = error else {
            panic!("应归类为 upstream_rejected");
        };
        assert_eq!(message.chars().count(), 200);
        assert!(message.chars().all(|character| character == '拒'));
    }

    #[tokio::test]
    async fn upstream_rejection_does_not_write_a_snapshot() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store.put(TOKEN_SLOT, b"test-token".as_slice()).unwrap();
        db.set_setting(SETTING_USER_ID, "user-1").unwrap();
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async {
                axum::Json(json!({
                    "success": false,
                    "message": "账户被禁用",
                    "data": null
                }))
            }),
        ))
        .await;
        db.set_setting(SETTING_BASE_URL, &base).unwrap();

        let error = refresh_account_usage_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("upstream_rejected"));
        assert_eq!(db.get_setting(SETTING_SNAPSHOT).unwrap(), None);
    }

    #[tokio::test]
    async fn cloudflare_html_is_invalid_response_and_never_panics() {
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=UTF-8")],
                    "<html><body>Just a moment... cf-challenge</body></html>",
                )
            }),
        ))
        .await;

        let error = fetch_self(&test_client(), &base, "token", "user-1")
            .await
            .unwrap_err();

        assert_eq!(error.code(), "invalid_response");
    }

    // ── §6.4:401/403 → authentication_failed,set 不落盘 ─────

    #[tokio::test]
    async fn auth_status_codes_are_classified_before_body_sniffing() {
        // 403 的 Cloudflare 挑战页(HTML)按状态码归为 authentication_failed,
        // 而不是 invalid_response;状态码优先,绝不去 HTML 里抠字段。
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=UTF-8")],
                    "<html><body>cf challenge</body></html>",
                )
            }),
        ))
        .await;
        let error = fetch_self(&test_client(), &base, "token", "user-1")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "authentication_failed");

        let unauthorized = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async { (StatusCode::UNAUTHORIZED, "unauthorized") }),
        ))
        .await;
        let error = fetch_self(&test_client(), &unauthorized, "token", "user-1")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "authentication_failed");
    }

    #[tokio::test]
    async fn set_fails_without_persisting_when_upstream_rejects_credentials() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async { (StatusCode::UNAUTHORIZED, "unauthorized") }),
        ))
        .await;

        let error = set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "sk-management-secret-123".to_string(),
            "user-1".to_string(),
            Some(base),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("authentication_failed"));
        // 不落盘:钥匙串与 settings 全部无变化。
        assert_eq!(store.get(TOKEN_SLOT).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_USER_ID).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_BASE_URL).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_SNAPSHOT).unwrap(), None);
    }

    // ── §6.5:网络错误 → connection_failed,旧快照保留 ────────

    #[tokio::test]
    async fn connection_failure_keeps_the_previous_snapshot_untouched() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store.put(TOKEN_SLOT, b"test-token".as_slice()).unwrap();
        db.set_setting(SETTING_USER_ID, "user-1").unwrap();

        // 先成功一次,留下旧快照。
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async {
                axum::Json(json!({
                    "success": true,
                    "message": "",
                    "data": { "quota": 61_400_000, "used_quota": 38_600_000 }
                }))
            }),
        ))
        .await;
        db.set_setting(SETTING_BASE_URL, &base).unwrap();
        let first = refresh_account_usage_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();
        let persisted_before = db.get_setting(SETTING_SNAPSHOT).unwrap().unwrap();

        // 端口占一次再关掉:必然 connection refused。
        let dead = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            address
        };
        db.set_setting(SETTING_BASE_URL, &format!("http://{dead}"))
            .unwrap();

        let error = refresh_account_usage_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("connection_failed"));
        // 失败不写快照:settings 里仍是字节相同的旧快照。
        assert_eq!(
            db.get_setting(SETTING_SNAPSHOT).unwrap().unwrap(),
            persisted_before
        );
        assert_eq!(read_snapshot(&db).unwrap(), Some(first));
    }

    // ── §6.6:调度 ──────────────────────────────────────────

    #[test]
    fn failure_retry_delay_maps_the_ladder_and_caps_at_the_last_step() {
        assert_eq!(
            failure_retry_delay(0, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(30)
        );
        assert_eq!(
            failure_retry_delay(1, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(60)
        );
        assert_eq!(
            failure_retry_delay(2, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(300)
        );
        assert_eq!(
            failure_retry_delay(3, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(600)
        );
        assert_eq!(
            failure_retry_delay(4, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(1_800)
        );
        assert_eq!(
            failure_retry_delay(9, &FAILURE_RETRY_DELAYS),
            Duration::from_secs(1_800)
        );
    }

    struct FakeDependencies {
        refresh_calls: AtomicUsize,
        refresh_results: Mutex<VecDeque<Result<(), AppError>>>,
        lightweight: AtomicBool,
        configured: AtomicBool,
        pending: AtomicBool,
    }

    impl FakeDependencies {
        fn new(configured: bool) -> Self {
            Self {
                refresh_calls: AtomicUsize::new(0),
                refresh_results: Mutex::new(VecDeque::new()),
                lightweight: AtomicBool::new(false),
                configured: AtomicBool::new(configured),
                pending: AtomicBool::new(false),
            }
        }
    }

    impl PackyCodeUsageDependencies for FakeDependencies {
        fn refresh(&self) -> RefreshFuture<'_> {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            if self.pending.load(Ordering::SeqCst) {
                return Box::pin(std::future::pending());
            }
            let result = self
                .refresh_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(()));
            Box::pin(async move { result })
        }

        fn is_lightweight_mode(&self) -> bool {
            self.lightweight.load(Ordering::SeqCst)
        }

        fn has_credentials(&self) -> bool {
            self.configured.load(Ordering::SeqCst)
        }
    }

    async fn wait_for_calls(dependencies: &FakeDependencies, at_least: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while dependencies.refresh_calls.load(Ordering::SeqCst) < at_least {
            assert!(tokio::time::Instant::now() < deadline, "等待刷新次数超时");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    // 调度测试用的短周期/短退避,必须是 const 项:spawn 的 future 要 'static,
    // 函数内临时数组的引用无法晋升为 'static。
    const TICK_10MS: [Duration; 1] = [Duration::from_millis(10)];
    const RETRY_30_60MS: [Duration; 2] = [Duration::from_millis(30), Duration::from_millis(60)];

    #[tokio::test]
    async fn scheduler_waits_a_full_interval_between_successful_refreshes() {
        let dependencies = Arc::new(FakeDependencies::new(true));
        let handle = start_scheduler_with_dependencies(
            dependencies.clone(),
            Duration::from_millis(200),
            &TICK_10MS,
        );

        // 启动即刷新一次,成功后必须等满一个周期才允许下一次。
        wait_for_calls(&dependencies, 1).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(dependencies.refresh_calls.load(Ordering::SeqCst), 1);
        wait_for_calls(&dependencies, 2).await;

        handle.stop().await;
    }

    #[tokio::test]
    async fn scheduler_retries_failed_refreshes_after_the_backoff_delay() {
        let dependencies = Arc::new(FakeDependencies::new(true));
        dependencies.refresh_results.lock().unwrap().extend([
            Err(AppError::Message("connection_failed".to_string())),
            Err(AppError::Message("connection_failed".to_string())),
            Ok(()),
        ]);
        let handle = start_scheduler_with_dependencies(
            dependencies.clone(),
            Duration::from_secs(60 * 60),
            &RETRY_30_60MS,
        );

        wait_for_calls(&dependencies, 3).await;
        assert_eq!(dependencies.refresh_calls.load(Ordering::SeqCst), 3);

        handle.stop().await;
    }

    #[tokio::test]
    async fn scheduler_stop_cancels_a_started_tick_promptly() {
        let dependencies = Arc::new(FakeDependencies::new(true));
        dependencies.pending.store(true, Ordering::SeqCst);
        let handle = start_scheduler_with_dependencies(
            dependencies,
            Duration::from_secs(60 * 60),
            &FAILURE_RETRY_DELAYS,
        );
        tokio::task::yield_now().await;

        tokio::time::timeout(Duration::from_millis(250), handle.stop())
            .await
            .expect("stop 不应等待挂起的刷新请求");
    }

    #[tokio::test]
    async fn lightweight_or_unconfigured_scheduler_does_no_work() {
        let dependencies = Arc::new(FakeDependencies::new(true));
        dependencies.lightweight.store(true, Ordering::SeqCst);
        let handle = start_scheduler_with_dependencies(
            dependencies.clone(),
            Duration::from_millis(20),
            &TICK_10MS,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(dependencies.refresh_calls.load(Ordering::SeqCst), 0);
        handle.stop().await;

        let unconfigured = Arc::new(FakeDependencies::new(false));
        let handle = start_scheduler_with_dependencies(
            unconfigured.clone(),
            Duration::from_millis(20),
            &TICK_10MS,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(unconfigured.refresh_calls.load(Ordering::SeqCst), 0);
        handle.stop().await;
    }

    // ── §6.7:未配置凭据 ────────────────────────────────────

    #[test]
    fn view_without_credentials_is_all_none_and_has_credentials_false() {
        let db = memory_db();
        let store = MemoryCredentialStore::default();

        let view = account_usage_view(&db, &store).unwrap();

        assert_eq!(
            view,
            PackyCodeAccountUsageView {
                balance_usd: None,
                used_usd: None,
                reference_usd: None,
                quota_per_unit: None,
                fetched_at: None,
                has_credentials: false,
            }
        );
    }

    // ── §6.8:set/clear 与令牌不泄密 ─────────────────────────

    #[tokio::test]
    async fn set_then_clear_round_trips_credentials_and_snapshot() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        let base = spawn_server(
            Router::new()
                .route(
                    "/api/status",
                    get(|| async { axum::Json(json!({ "quota_per_unit": 500_000 })) }),
                )
                .route(
                    "/api/user/self",
                    get(|| async {
                        axum::Json(json!({
                            "success": true,
                            "message": "",
                            "data": { "quota": 30_700_000, "used_quota": 19_300_000 }
                        }))
                    }),
                ),
        )
        .await;

        set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "sk-management-secret-123".to_string(),
            "user-1".to_string(),
            Some(base.clone()),
        )
        .await
        .unwrap();

        // set 后:钥匙串 + settings 有值。
        assert_eq!(
            store.get(TOKEN_SLOT).unwrap().unwrap(),
            b"sk-management-secret-123"
        );
        assert_eq!(db.get_setting(SETTING_USER_ID).unwrap().unwrap(), "user-1");
        assert_eq!(
            db.get_setting(SETTING_BASE_URL).unwrap().unwrap(),
            base,
            "base_url 应按写入值落盘"
        );

        // 视图只看得到状态,看不到令牌。
        let view = account_usage_view(&db, store.as_ref()).unwrap();
        assert!(view.has_credentials);
        assert_eq!(view.balance_usd, None, "set 本身不写快照");

        // 刷新一次:视图有值,且快照/视图序列化都不含令牌明文。
        let refreshed = manual_refresh_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();
        assert_eq!(refreshed.balance_usd.as_deref(), Some("61.4"));
        assert_eq!(refreshed.used_usd.as_deref(), Some("38.6"));
        assert_eq!(refreshed.reference_usd.as_deref(), Some("100"));
        let view_json = serde_json::to_string(&refreshed).unwrap();
        assert!(!view_json.contains("sk-management-secret-123"));
        assert!(!view_json.contains("sk-management"));

        // 60 秒内重复刷新复用上次快照(不重复请求)。
        let reused = manual_refresh_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();
        assert_eq!(reused.fetched_at, refreshed.fetched_at);

        clear_credentials(&db, store.as_ref()).unwrap();
        assert_eq!(store.get(TOKEN_SLOT).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_USER_ID).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_BASE_URL).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_SNAPSHOT).unwrap(), None);
        assert!(
            !account_usage_view(&db, store.as_ref())
                .unwrap()
                .has_credentials
        );
    }

    #[tokio::test]
    async fn error_strings_never_contain_the_token_plaintext() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(|| async { (StatusCode::UNAUTHORIZED, "unauthorized") }),
        ))
        .await;

        let error = set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "sk-usage-secret-abc123xyz".to_string(),
            "user-1".to_string(),
            Some(base),
        )
        .await
        .unwrap_err();

        let message = error.to_string();
        assert!(!message.contains("sk-usage-secret-abc123xyz"));
        assert!(!message.contains("abc123xyz"));

        // 空令牌/空用户 ID 的错误同样不含任何输入值。
        let error = set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "sk-usage-secret-abc123xyz".to_string(),
            "".to_string(),
            None,
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("abc123xyz"));
    }

    #[tokio::test]
    async fn set_rejects_empty_token_or_user_id_without_any_request() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(move || {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                async {
                    axum::Json(json!({
                        "success": true,
                        "message": "",
                        "data": { "quota": 1, "used_quota": 0 }
                    }))
                }
            }),
        ))
        .await;

        let error = set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "  ".to_string(),
            "user-1".to_string(),
            Some(base.clone()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("不能为空"));
        assert_eq!(store.get(TOKEN_SLOT).unwrap(), None);
        assert_eq!(db.get_setting(SETTING_USER_ID).unwrap(), None);

        let error = set_credentials_with_client(
            &test_client(),
            &db,
            store.as_ref(),
            "token".to_string(),
            "  ".to_string(),
            Some(base),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("不能为空"));
        assert_eq!(hits.load(Ordering::SeqCst), 0, "空输入不该发请求");
    }

    #[tokio::test]
    async fn request_headers_carry_both_token_and_user_id() {
        let headers_seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let headers_clone = headers_seen.clone();
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(move |request: axum::extract::Request| {
                let mut seen = headers_clone.lock().unwrap();
                seen.push(
                    request
                        .headers()
                        .get("authorization")
                        .map(|value| value.to_str().unwrap().to_string())
                        .unwrap_or_default(),
                );
                seen.push(
                    request
                        .headers()
                        .get("new-api-user")
                        .map(|value| value.to_str().unwrap().to_string())
                        .unwrap_or_default(),
                );
                async {
                    axum::Json(json!({
                        "success": true,
                        "message": "",
                        "data": { "quota": 10, "used_quota": 1 }
                    }))
                }
            }),
        ))
        .await;

        fetch_self(&test_client(), &base, "token-abc", "user-42")
            .await
            .unwrap();

        let seen = headers_seen.lock().unwrap();
        assert_eq!(seen[0], "Bearer token-abc");
        assert_eq!(seen[1], "user-42");
    }

    #[tokio::test]
    async fn manual_refresh_within_60_seconds_reuses_the_snapshot() {
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store.put(TOKEN_SLOT, b"test-token".as_slice()).unwrap();
        db.set_setting(SETTING_USER_ID, "user-1").unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        let base = spawn_server(Router::new().route(
            "/api/user/self",
            get(move || {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                async {
                    axum::Json(json!({
                        "success": true,
                        "message": "",
                        "data": { "quota": 10, "used_quota": 1 }
                    }))
                }
            }),
        ))
        .await;
        db.set_setting(SETTING_BASE_URL, &base).unwrap();

        let first = manual_refresh_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();
        let second = manual_refresh_with_client(&test_client(), &db, store.as_ref())
            .await
            .unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1, "60 秒内不该重复请求");
        assert_eq!(second.fetched_at, first.fetched_at);
    }

    // 直接调 handler 的解析测试:200 + HTML 防护页 → invalid_response。
    #[tokio::test]
    async fn html_on_ok_status_is_invalid_response_via_document_parser() {
        let response = call_handler(
            Router::new().route(
                "/api/user/self",
                get(|| async {
                    (
                        StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "text/html; charset=UTF-8")],
                        "<html><body>Just a moment...</body></html>",
                    )
                }),
            ),
            Request::builder()
                .uri("/api/user/self")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        let error = parse_self_document(&bytes).unwrap_err();
        assert_eq!(error.code(), "invalid_response");
    }
}
