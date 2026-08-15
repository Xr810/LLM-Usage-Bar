//! 供应商余额查询服务
//!
//! 支持 DeepSeek、StepFun、SiliconFlow、OpenRouter、Novita AI 的账户余额查询。
//! 返回 UsageResult 格式，与现有用量系统无缝对接。
//!
//! 错误通道语义（与 coding_plan / subscription 两个服务保持一致）：
//! - `Err(String)` = 瞬时传输失败（网络不可达/超时/读体中断）。前端 invoke reject，
//!   react-query 触发 retry 并保留上一次成功的 data（天然 keep-last-good）。
//! - `Ok(success:false)` = 确定性失败（空 key/未知供应商/鉴权/非 2xx/响应体非法 JSON），
//!   立即透出错误文案。判定按 reqwest 错误种类在折叠点完成，不依赖错误文案匹配。

use crate::error::AppError;
use crate::provider::{UsageData, UsageResult};
use crate::secrets::CredentialStore;
use crate::store::{lock_conn, Database};
use rusqlite::params;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

// ── 供应商检测 ──────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum BalanceProvider {
    DeepSeek,
    StepFun,
    SiliconFlow,
    SiliconFlowEn,
    OpenRouter,
    NovitaAI,
}

fn detect_provider(base_url: &str) -> Option<BalanceProvider> {
    let url = base_url.to_lowercase();
    if url.contains("api.deepseek.com") {
        Some(BalanceProvider::DeepSeek)
    } else if url.contains("api.stepfun.ai") || url.contains("api.stepfun.com") {
        Some(BalanceProvider::StepFun)
    } else if url.contains("api.siliconflow.cn") {
        Some(BalanceProvider::SiliconFlow)
    } else if url.contains("api.siliconflow.com") {
        Some(BalanceProvider::SiliconFlowEn)
    } else if url.contains("openrouter.ai") {
        Some(BalanceProvider::OpenRouter)
    } else if url.contains("api.novita.ai") {
        Some(BalanceProvider::NovitaAI)
    } else {
        None
    }
}

fn make_error(msg: String) -> UsageResult {
    UsageResult {
        success: false,
        data: None,
        error: Some(msg),
    }
}

fn make_auth_error(status: reqwest::StatusCode) -> UsageResult {
    UsageResult {
        success: false,
        data: Some(vec![UsageData {
            plan_name: None,
            remaining: None,
            total: None,
            used: None,
            unit: None,
            is_valid: Some(false),
            invalid_message: Some(format!("Authentication failed (HTTP {status})")),
            extra: None,
        }]),
        error: Some(format!("Authentication failed (HTTP {status})")),
    }
}

// ── DeepSeek ────────────────────────────────────────────────
// GET https://api.deepseek.com/user/balance
// Response: { balance_infos: [{ currency, total_balance, granted_balance, topped_up_balance }], is_available }

async fn query_deepseek(api_key: &str) -> Result<UsageResult, String> {
    let client = crate::http_client::get();

    let resp = client
        .get("https://api.deepseek.com/user/balance")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(make_auth_error(status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    // 先 bytes() 再解析：读体失败（超时/连接中断）是瞬时 → Err；拿到完整响应体
    // 后解析失败才是确定性。reqwest 的 json() 把读体错误也包成 decode，无法区分。
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    let is_available = body
        .get("is_available")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut data = Vec::new();

    if let Some(infos) = body.get("balance_infos").and_then(|v| v.as_array()) {
        for info in infos {
            let currency = info
                .get("currency")
                .and_then(|v| v.as_str())
                .unwrap_or("CNY");
            let total = parse_f64_field(info, "total_balance");

            data.push(UsageData {
                plan_name: Some(currency.to_string()),
                remaining: total,
                total: None,
                used: None,
                unit: Some(currency.to_string()),
                is_valid: Some(is_available),
                invalid_message: if !is_available {
                    Some("Insufficient balance".to_string())
                } else {
                    None
                },
                extra: None,
            });
        }
    }

    Ok(UsageResult {
        success: true,
        data: if data.is_empty() { None } else { Some(data) },
        error: None,
    })
}

// ── StepFun ─────────────────────────────────────────────────
// GET https://api.stepfun.com/v1/accounts
// Response: { object, type, balance, total_cash_balance, total_voucher_balance }

async fn query_stepfun(api_key: &str) -> Result<UsageResult, String> {
    let client = crate::http_client::get();

    let resp = client
        .get("https://api.stepfun.com/v1/accounts")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(make_auth_error(status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    // 先 bytes() 再解析：读体失败（超时/连接中断）是瞬时 → Err；拿到完整响应体
    // 后解析失败才是确定性。reqwest 的 json() 把读体错误也包成 decode，无法区分。
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    let balance = parse_f64_field(&body, "balance").unwrap_or(0.0);

    Ok(UsageResult {
        success: true,
        data: Some(vec![UsageData {
            plan_name: Some("StepFun".to_string()),
            remaining: Some(balance),
            total: None,
            used: None,
            unit: Some("CNY".to_string()),
            is_valid: Some(true),
            invalid_message: None,
            extra: None,
        }]),
        error: None,
    })
}

// ── SiliconFlow ─────────────────────────────────────────────
// GET https://api.siliconflow.cn/v1/user/info (or .com for EN)
// Response: { code, data: { balance, chargeBalance, totalBalance, status } }

async fn query_siliconflow(api_key: &str, is_cn: bool) -> Result<UsageResult, String> {
    let client = crate::http_client::get();

    let domain = if is_cn {
        "api.siliconflow.cn"
    } else {
        "api.siliconflow.com"
    };
    let url = format!("https://{domain}/v1/user/info");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(make_auth_error(status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    // 先 bytes() 再解析：读体失败（超时/连接中断）是瞬时 → Err；拿到完整响应体
    // 后解析失败才是确定性。reqwest 的 json() 把读体错误也包成 decode，无法区分。
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    let data = match body.get("data") {
        Some(d) => d,
        None => return Ok(make_error("Missing 'data' field in response".to_string())),
    };

    let total_balance = parse_f64_field(data, "totalBalance").unwrap_or(0.0);

    let unit = if is_cn { "CNY" } else { "USD" };
    let plan_name = if is_cn {
        "SiliconFlow"
    } else {
        "SiliconFlow (EN)"
    };

    Ok(UsageResult {
        success: true,
        data: Some(vec![UsageData {
            plan_name: Some(plan_name.to_string()),
            remaining: Some(total_balance),
            total: None,
            used: None,
            unit: Some(unit.to_string()),
            is_valid: Some(true),
            invalid_message: None,
            extra: None,
        }]),
        error: None,
    })
}

// ── OpenRouter(账户余额,D1 改造)────────────────────────────
// GET https://openrouter.ai/api/v1/credits
// 成功:{ "data": { "total_credits": 100.5, "total_usage": 25.75 } }
// total_credits = 累计充值(参考额度),total_usage = 累计消费,余额 = 前者 − 后者。
//
// 此端点只认 Management API key:普通 sk-or-* 模型 key 会得到 403
// ("Only management keys can perform this operation")。因此这里的凭据来源
// 固定为钥匙串 slot "openrouter-management-key",绝不用 provider 存的模型 key 回退;
// 403 会被明确映射成 authentication_failed,而不是静默当 0 或换 key 重试。

pub const OPENROUTER_MANAGEMENT_KEY_SLOT: &str = "openrouter-management-key";
pub const OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY: &str = "openrouter_account_balance";

/// 15 分钟刷新间隔,与 provider key usage 调度一致。
const OPENROUTER_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// 失败退避阶梯:失败保留旧快照,按阶梯延迟重试;连续失败封顶在最后一档。
const OPENROUTER_FAILURE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(600),
    Duration::from_secs(1_800),
];

/// 手动刷新防连点:60 秒内重复调用复用上次快照(与 packycode 用量同一口径)。
const OPENROUTER_MANUAL_REFRESH_MIN_INTERVAL_SECS: i64 = 60;
/// 上游 body 进错误串前截断,防超长文本刷屏日志。
const OPENROUTER_UPSTREAM_BODY_MAX_CHARS: usize = 200;

/// 上游查询失败的四类错误码。Display 绝不包含 key 明文;文案与本文件
/// 另外四家保持同一种语言(英文),别让同一个面板一半中文一半英文。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenRouterBalanceError {
    #[error("Authentication error (HTTP {0})")]
    AuthenticationFailed(u16),
    #[error("Network error: {0}")]
    ConnectionFailed(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    /// 非 2xx。带上(截断过的)上游 body —— 只有 HTTP 码没法诊断。
    #[error("API error (HTTP {status}): {body}")]
    UpstreamRejected { status: u16, body: String },
}

impl From<OpenRouterBalanceError> for AppError {
    fn from(error: OpenRouterBalanceError) -> Self {
        AppError::Message(error.to_string())
    }
}

fn truncate_upstream_body(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .take(OPENROUTER_UPSTREAM_BODY_MAX_CHARS)
        .collect()
}

/// 一次成功查询的结果。金额一律十进制字符串(仓库约定:金额不 round)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRouterCreditsPayload {
    pub total_credits_usd: String,
    pub total_usage_usd: String,
    pub balance_usd: String,
}

/// 持久化快照:settings 键 `openrouter_account_balance` 的 JSON 形态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRouterBalanceSnapshot {
    pub total_credits_usd: String,
    pub total_usage_usd: String,
    pub balance_usd: String,
    pub fetched_at: i64,
}

/// 对外视图。无快照时金额全 None(面板自己决定怎么显示),绝不编造 0。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterAccountBalanceView {
    pub total_credits_usd: Option<String>,
    pub total_usage_usd: Option<String>,
    pub balance_usd: Option<String>,
    pub fetched_at: Option<i64>,
    pub has_management_key: bool,
}

/// 网络查询(生产路径,8 秒超时)。读体失败(超时/连接中断)算 connection_failed。
async fn query_openrouter(
    api_key: &str,
) -> Result<OpenRouterCreditsPayload, OpenRouterBalanceError> {
    let client = crate::http_client::get();

    let resp = client
        .get("https://openrouter.ai/api/v1/credits")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(8))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(OpenRouterBalanceError::ConnectionFailed(e.to_string())),
    };

    let status = resp.status().as_u16();
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return Err(OpenRouterBalanceError::ConnectionFailed(format!(
                "Failed to read response: {e}"
            )))
        }
    };
    parse_openrouter_response(status, &raw)
}

/// 状态码分类 + 200 响应体解析,拆成纯函数便于不依赖网络的单测。
fn parse_openrouter_response(
    status: u16,
    raw: &[u8],
) -> Result<OpenRouterCreditsPayload, OpenRouterBalanceError> {
    if status == 401 || status == 403 {
        return Err(OpenRouterBalanceError::AuthenticationFailed(status));
    }
    if !(200..300).contains(&status) {
        return Err(OpenRouterBalanceError::UpstreamRejected {
            status,
            body: truncate_upstream_body(raw),
        });
    }

    let body: serde_json::Value = serde_json::from_slice(raw).map_err(|e| {
        OpenRouterBalanceError::InvalidResponse(format!("Failed to parse response: {e}"))
    })?;
    parse_openrouter_credits(&body)
}

/// 只认 `data.total_credits` / `data.total_usage`;缺失/非数字/负数都是
/// invalid_response——无价 ≠ 免费,绝不落 0。余额 = 充值 − 消费。
fn parse_openrouter_credits(
    body: &serde_json::Value,
) -> Result<OpenRouterCreditsPayload, OpenRouterBalanceError> {
    let data = body.get("data").ok_or_else(|| {
        OpenRouterBalanceError::InvalidResponse("Missing 'data' field in response".to_string())
    })?;
    let total_credits = parse_decimal_field(data, "total_credits")?;
    let total_usage = parse_decimal_field(data, "total_usage")?;
    let balance = total_credits - total_usage;

    Ok(OpenRouterCreditsPayload {
        total_credits_usd: total_credits.normalize().to_string(),
        total_usage_usd: total_usage.normalize().to_string(),
        balance_usd: balance.normalize().to_string(),
    })
}

/// 把字段解析成 rust_decimal::Decimal(兼容 JSON 数字与字符串形式),
/// 缺失/非数字/负数都报 invalid_response。
fn parse_decimal_field(
    obj: &serde_json::Value,
    field: &str,
) -> Result<Decimal, OpenRouterBalanceError> {
    let value = obj.get(field).ok_or_else(|| {
        OpenRouterBalanceError::InvalidResponse(format!("Missing '{field}' field in response"))
    })?;
    let raw = match value {
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text.clone(),
        _ => {
            return Err(OpenRouterBalanceError::InvalidResponse(format!(
                "'{field}' is not a number"
            )))
        }
    };
    let decimal = Decimal::from_str(&raw).map_err(|_| {
        OpenRouterBalanceError::InvalidResponse(format!("'{field}' is not a valid number"))
    })?;
    if decimal.is_sign_negative() {
        return Err(OpenRouterBalanceError::InvalidResponse(format!(
            "'{field}' is negative"
        )));
    }
    Ok(decimal)
}

/// 旧的 per-provider 余额视图与新的账户级查询之间的适配:金额在内部已是
/// 十进制字符串,转 f64 只为喂给旧视图,解析失败给 None,绝不编造 0。
fn openrouter_payload_to_usage_result(payload: OpenRouterCreditsPayload) -> UsageResult {
    let remaining = payload.balance_usd.parse::<f64>().ok();
    UsageResult {
        success: true,
        data: Some(vec![UsageData {
            plan_name: Some("OpenRouter".to_string()),
            remaining,
            total: payload.total_credits_usd.parse::<f64>().ok(),
            used: payload.total_usage_usd.parse::<f64>().ok(),
            unit: Some("USD".to_string()),
            is_valid: Some(remaining.is_some_and(|value| value > 0.0)),
            invalid_message: if remaining.is_some_and(|value| value <= 0.0) {
                Some("No credits remaining".to_string())
            } else {
                None
            },
            extra: None,
        }]),
        error: None,
    }
}

// ── OpenRouter 存储层(方案 A:settings 快照 + 钥匙串 key,不建表)──

/// 读快照。settings 里没有键 → None;JSON 损坏 → 明确报错,不静默当无。
pub fn load_snapshot(db: &Database) -> Result<Option<OpenRouterBalanceSnapshot>, AppError> {
    let Some(raw) = db.get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)? else {
        return Ok(None);
    };
    let snapshot: OpenRouterBalanceSnapshot = serde_json::from_str(&raw).map_err(|error| {
        AppError::Config(format!(
            "invalid {OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY} setting: {error}"
        ))
    })?;
    Ok(Some(snapshot))
}

/// 写快照(整值替换)。
pub fn save_snapshot(db: &Database, snapshot: &OpenRouterBalanceSnapshot) -> Result<(), AppError> {
    let raw = serde_json::to_string(snapshot)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    db.set_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY, &raw)
}

/// 删快照(clear 命令用)。
pub fn clear_snapshot(db: &Database) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    conn.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY],
    )
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// 管理 key 是否已配置:get 返回 Some 且非空。
pub fn has_openrouter_management_key(credentials: &dyn CredentialStore) -> bool {
    matches!(
        credentials.get(OPENROUTER_MANAGEMENT_KEY_SLOT),
        Ok(Some(bytes)) if !bytes.is_empty()
    )
}

/// 读管理 key。None = 未配置(空串/全空白也算未配置)。
/// 调度器与手动刷新共用这一份读法,不各写一遍。
fn read_openrouter_management_key(
    credentials: &dyn CredentialStore,
) -> Result<Option<String>, AppError> {
    match credentials.get(OPENROUTER_MANAGEMENT_KEY_SLOT) {
        Ok(Some(bytes)) => Ok(String::from_utf8(bytes)
            .ok()
            .filter(|key| !key.trim().is_empty())),
        Ok(None) => Ok(None),
        Err(error) => Err(AppError::Message(format!(
            "credential store read failed: {error}"
        ))),
    }
}

/// 手动刷新:立刻查一次并落快照。
///
/// 调度器只在每轮开头读一次凭据,所以刚设完 key 最长要等一个刷新周期;
/// 这个入口让「设完就能看见」不必等 15 分钟,也让退避期间改对的 key 立刻生效。
/// 60 秒内重复调用直接复用上次快照(防连点),未配置 key 直接报错、不发请求。
pub async fn manual_refresh_openrouter_balance(
    db: &Database,
    credentials: &dyn CredentialStore,
) -> Result<(), AppError> {
    if let Some(snapshot) = load_snapshot(db)? {
        if unix_timestamp()?.saturating_sub(snapshot.fetched_at)
            < OPENROUTER_MANUAL_REFRESH_MIN_INTERVAL_SECS
        {
            log::debug!("openrouter balance refresh throttled, reusing the last snapshot");
            return Ok(());
        }
    }

    let Some(api_key) = read_openrouter_management_key(credentials)? else {
        return Err(AppError::InvalidInput(
            "OpenRouter 管理 key 未配置".to_string(),
        ));
    };

    let payload = query_openrouter(&api_key).await?;
    save_snapshot(
        db,
        &OpenRouterBalanceSnapshot {
            total_credits_usd: payload.total_credits_usd,
            total_usage_usd: payload.total_usage_usd,
            balance_usd: payload.balance_usd,
            fetched_at: unix_timestamp()?,
        },
    )?;
    crate::usage_events::notify_dashboard_invalidated();
    log::info!("openrouter account balance refreshed on demand");
    Ok(())
}

// ── OpenRouter 调度器(照 official_pricing.rs 的模板)─────────

type OpenRouterFetchFuture<'a> = Pin<
    Box<dyn Future<Output = Result<OpenRouterCreditsPayload, OpenRouterBalanceError>> + Send + 'a>,
>;

/// 调度器依赖的注入面:网络与存储都可替换,便于不依赖真实钥匙串/网络的单测。
trait OpenRouterBalanceSchedulerDependencies: Send + Sync {
    /// 当前管理 key(None = 未配置,调度空转)。
    fn management_key(&self) -> Result<Option<String>, AppError>;
    /// 带管理 key 的网络查询。
    fn fetch_credits(&self, api_key: &str) -> OpenRouterFetchFuture<'_>;
    /// 落盘快照。失败路径不调用它,旧快照因此原样保留。
    fn save_snapshot(&self, snapshot: &OpenRouterBalanceSnapshot) -> Result<(), AppError>;
    fn is_lightweight_mode(&self) -> bool;
    fn notify_dashboard_invalidated(&self);
}

struct ProductionOpenRouterBalanceDependencies {
    db: Arc<Database>,
    credentials: Arc<dyn CredentialStore>,
}

impl OpenRouterBalanceSchedulerDependencies for ProductionOpenRouterBalanceDependencies {
    fn management_key(&self) -> Result<Option<String>, AppError> {
        read_openrouter_management_key(&*self.credentials)
    }

    fn fetch_credits(&self, api_key: &str) -> OpenRouterFetchFuture<'_> {
        let api_key = api_key.to_string();
        Box::pin(async move { query_openrouter(&api_key).await })
    }

    fn save_snapshot(&self, snapshot: &OpenRouterBalanceSnapshot) -> Result<(), AppError> {
        save_snapshot(&self.db, snapshot)
    }

    fn is_lightweight_mode(&self) -> bool {
        crate::lightweight::is_lightweight_mode()
    }

    fn notify_dashboard_invalidated(&self) {
        crate::usage_events::notify_dashboard_invalidated();
    }
}

pub struct OpenRouterBalanceSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl OpenRouterBalanceSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for OpenRouterBalanceSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn start_openrouter_balance_scheduler(
    db: Arc<Database>,
    credentials: Arc<dyn CredentialStore>,
) -> OpenRouterBalanceSchedulerHandle {
    start_openrouter_balance_scheduler_with_dependencies(
        Arc::new(ProductionOpenRouterBalanceDependencies { db, credentials }),
        OPENROUTER_REFRESH_INTERVAL,
    )
}

fn start_openrouter_balance_scheduler_with_dependencies(
    dependencies: Arc<dyn OpenRouterBalanceSchedulerDependencies>,
    refresh_interval: Duration,
) -> OpenRouterBalanceSchedulerHandle {
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let task = tauri::async_runtime::spawn(async move {
        if !refresh_with_backoff_or_cancel(
            &*dependencies,
            &OPENROUTER_FAILURE_RETRY_DELAYS,
            &mut cancel_rx,
        )
        .await
        {
            return;
        }

        loop {
            if !wait_or_cancel(refresh_interval, &mut cancel_rx).await {
                return;
            }
            if !refresh_with_backoff_or_cancel(
                &*dependencies,
                &OPENROUTER_FAILURE_RETRY_DELAYS,
                &mut cancel_rx,
            )
            .await
            {
                return;
            }
        }
    });
    OpenRouterBalanceSchedulerHandle {
        cancel_tx,
        task: Some(task),
    }
}

async fn refresh_with_backoff_or_cancel(
    dependencies: &dyn OpenRouterBalanceSchedulerDependencies,
    delays: &[Duration],
    cancel_rx: &mut watch::Receiver<bool>,
) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    // 取消走到 changed 分支:整个 refresh future(挂起的 fetch 或退避睡眠)被
    // select 丢弃,因此取消在挂起请求与退避等待期间都即时生效。
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = refresh_with_backoff(dependencies, delays) => true,
    }
}

/// 一轮刷新 + 失败退避。无管理 key / 轻量模式直接空转(不算失败、不请求);
/// 成功才写快照并通知面板,失败保留旧快照不动并按阶梯延迟重试。
/// 自身不处理取消:取消由外层 select 丢弃整个 future 完成。
async fn refresh_with_backoff(
    dependencies: &dyn OpenRouterBalanceSchedulerDependencies,
    delays: &[Duration],
) {
    if dependencies.is_lightweight_mode() {
        return;
    }
    let api_key = match dependencies.management_key() {
        Ok(Some(key)) => key,
        Ok(None) => return,
        Err(error) => {
            log::warn!("openrouter balance scheduler could not read the management key: {error}");
            return;
        }
    };

    let mut failure_count = 0usize;
    loop {
        match dependencies.fetch_credits(&api_key).await {
            Ok(payload) => {
                let save_result = unix_timestamp().and_then(|fetched_at| {
                    dependencies.save_snapshot(&OpenRouterBalanceSnapshot {
                        total_credits_usd: payload.total_credits_usd,
                        total_usage_usd: payload.total_usage_usd,
                        balance_usd: payload.balance_usd,
                        fetched_at,
                    })
                });
                match save_result {
                    Ok(()) => {
                        log::info!("openrouter account balance refreshed");
                        dependencies.notify_dashboard_invalidated();
                        return;
                    }
                    Err(error) => {
                        log::warn!("openrouter balance snapshot save failed: {error}");
                    }
                }
            }
            Err(error) => {
                log::warn!("openrouter balance refresh failed: {error}");
            }
        }

        let delay = openrouter_failure_retry_delay(delays, failure_count);
        failure_count = failure_count.saturating_add(1);
        tokio::time::sleep(delay).await;
    }
}

/// 第 n 次(从 0 计)失败后的重试延迟,封顶在最后一档。
fn openrouter_failure_retry_delay(delays: &[Duration], failure_count: usize) -> Duration {
    delays[failure_count.min(delays.len().saturating_sub(1))]
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

// ── Novita AI ───────────────────────────────────────────────
// GET https://api.novita.ai/v3/user/balance
// Response: { availableBalance, cashBalance, creditLimit, outstandingInvoices }
// 金额单位：0.0001 USD

async fn query_novita(api_key: &str) -> Result<UsageResult, String> {
    let client = crate::http_client::get();

    let resp = client
        .get("https://api.novita.ai/v3/user/balance")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(make_auth_error(status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(make_error(format!("API error (HTTP {status}): {body}")));
    }

    // 先 bytes() 再解析：读体失败（超时/连接中断）是瞬时 → Err；拿到完整响应体
    // 后解析失败才是确定性。reqwest 的 json() 把读体错误也包成 decode，无法区分。
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return Ok(make_error(format!("Failed to parse response: {e}"))),
    };

    // Novita 金额单位为 0.0001 USD，需除以 10000 转为 USD
    let available = parse_f64_field(&body, "availableBalance").unwrap_or(0.0) / 10000.0;

    Ok(UsageResult {
        success: true,
        data: Some(vec![UsageData {
            plan_name: Some("Novita AI".to_string()),
            remaining: Some(available),
            total: None,
            used: None,
            unit: Some("USD".to_string()),
            is_valid: Some(available > 0.0),
            invalid_message: if available <= 0.0 {
                Some("No balance remaining".to_string())
            } else {
                None
            },
            extra: None,
        }]),
        error: None,
    })
}

// ── 工具函数 ────────────────────────────────────────────────

/// 解析 JSON 字段为 f64，兼容数字和字符串格式
fn parse_f64_field(obj: &serde_json::Value, field: &str) -> Option<f64> {
    obj.get(field).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

// ── 公开入口 ────────────────────────────────────────────────

/// 查询余额。瞬时传输失败返回 `Err`（前端 reject → retry + 保留上次成功值），
/// 确定性失败返回 `Ok(success:false)`（见模块级文档）。
pub async fn get_balance(base_url: &str, api_key: &str) -> Result<UsageResult, String> {
    if api_key.trim().is_empty() {
        return Ok(UsageResult {
            success: false,
            data: None,
            error: Some("API key is empty".to_string()),
        });
    }

    let provider = match detect_provider(base_url) {
        Some(p) => p,
        None => {
            return Ok(UsageResult {
                success: false,
                data: None,
                error: Some("Unknown balance provider".to_string()),
            })
        }
    };

    match provider {
        BalanceProvider::DeepSeek => query_deepseek(api_key).await,
        BalanceProvider::StepFun => query_stepfun(api_key).await,
        BalanceProvider::SiliconFlow => query_siliconflow(api_key, true).await,
        BalanceProvider::SiliconFlowEn => query_siliconflow(api_key, false).await,
        BalanceProvider::OpenRouter => match query_openrouter(api_key).await {
            Ok(payload) => Ok(openrouter_payload_to_usage_result(payload)),
            Err(OpenRouterBalanceError::AuthenticationFailed(status)) => Ok(make_auth_error(
                reqwest::StatusCode::from_u16(status).unwrap_or(reqwest::StatusCode::UNAUTHORIZED),
            )),
            Err(OpenRouterBalanceError::ConnectionFailed(message)) => {
                Err(format!("Network error: {message}"))
            }
            Err(error) => Ok(make_error(error.to_string())),
        },
        BalanceProvider::NovitaAI => query_novita(api_key).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::CredentialStoreError;
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ── 假凭据库(仅 put/get/delete,给无钥匙串的测试用)───────

    #[derive(Default)]
    struct MemoryCredentialStore {
        items: Mutex<HashMap<String, Vec<u8>>>,
        fail_next_put: AtomicBool,
    }

    impl CredentialStore for MemoryCredentialStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
            if self.fail_next_put.swap(false, Ordering::SeqCst) {
                return Err(CredentialStoreError::OperationFailed);
            }
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

    // ── 假调度依赖(网络可注入,存储走真实 settings 表)────────

    struct FakeOpenRouterDependencies {
        db: Arc<Database>,
        key: Mutex<Option<String>>,
        fetch_calls: AtomicUsize,
        keys_seen: Mutex<Vec<String>>,
        fetch_results: Mutex<VecDeque<Result<OpenRouterCreditsPayload, OpenRouterBalanceError>>>,
        pending_fetch: AtomicBool,
        lightweight: AtomicBool,
        notifications: AtomicUsize,
    }

    impl FakeOpenRouterDependencies {
        fn new() -> Self {
            Self {
                db: Arc::new(Database::memory().unwrap()),
                key: Mutex::new(None),
                fetch_calls: AtomicUsize::new(0),
                keys_seen: Mutex::new(Vec::new()),
                fetch_results: Mutex::new(VecDeque::new()),
                pending_fetch: AtomicBool::new(false),
                lightweight: AtomicBool::new(false),
                notifications: AtomicUsize::new(0),
            }
        }
    }

    impl OpenRouterBalanceSchedulerDependencies for FakeOpenRouterDependencies {
        fn management_key(&self) -> Result<Option<String>, AppError> {
            Ok(self.key.lock().unwrap().clone())
        }

        fn fetch_credits(&self, api_key: &str) -> OpenRouterFetchFuture<'_> {
            self.fetch_calls.fetch_add(1, Ordering::SeqCst);
            self.keys_seen.lock().unwrap().push(api_key.to_string());
            if self.pending_fetch.load(Ordering::SeqCst) {
                return Box::pin(std::future::pending());
            }
            let result = self
                .fetch_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(credits_payload("100.5", "25.75")));
            Box::pin(async move { result })
        }

        fn save_snapshot(&self, snapshot: &OpenRouterBalanceSnapshot) -> Result<(), AppError> {
            save_snapshot(&self.db, snapshot)
        }

        fn is_lightweight_mode(&self) -> bool {
            self.lightweight.load(Ordering::SeqCst)
        }

        fn notify_dashboard_invalidated(&self) {
            self.notifications.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn credits_payload(total_credits: &str, total_usage: &str) -> OpenRouterCreditsPayload {
        let total_credits = Decimal::from_str(total_credits).unwrap().normalize();
        let total_usage = Decimal::from_str(total_usage).unwrap().normalize();
        let balance = total_credits - total_usage;
        OpenRouterCreditsPayload {
            total_credits_usd: total_credits.to_string(),
            total_usage_usd: total_usage.to_string(),
            balance_usd: balance.to_string(),
        }
    }

    fn snapshot_fixture(
        total_credits: &str,
        total_usage: &str,
        balance: &str,
        fetched_at: i64,
    ) -> OpenRouterBalanceSnapshot {
        OpenRouterBalanceSnapshot {
            total_credits_usd: total_credits.to_string(),
            total_usage_usd: total_usage.to_string(),
            balance_usd: balance.to_string(),
            fetched_at,
        }
    }

    async fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not met within 5s"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// 跑到第一次失败(真实 30s 退避期间)就取消,返回该轮刷新的结果。
    async fn first_failure_then_cancel(dependencies: Arc<FakeOpenRouterDependencies>) -> bool {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let deps = dependencies.clone();
        let handle = tauri::async_runtime::spawn(async move {
            refresh_with_backoff_or_cancel(&*deps, &OPENROUTER_FAILURE_RETRY_DELAYS, &mut cancel_rx)
                .await
        });
        wait_for(|| dependencies.fetch_calls.load(Ordering::SeqCst) >= 1).await;
        let _ = cancel_tx.send(true);
        handle.await.unwrap()
    }

    // ── 1. 200 正常解析(含余额=充值−消费的减法断言)──────────

    #[test]
    fn parses_200_credits_response_and_computes_balance() {
        let payload = parse_openrouter_response(
            200,
            br#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#,
        )
        .unwrap();
        assert_eq!(payload.total_credits_usd, "100.5");
        assert_eq!(payload.total_usage_usd, "25.75");
        // 余额 = 充值 − 消费
        assert_eq!(payload.balance_usd, "74.75");
    }

    #[test]
    fn decimal_strings_avoid_binary_float_rounding() {
        let payload = parse_openrouter_response(
            200,
            br#"{"data":{"total_credits":"0.3","total_usage":"0.1"}}"#,
        )
        .unwrap();
        assert_eq!(payload.balance_usd, "0.2");
    }

    // ── 2. 401/403 → authentication_failed,且不写快照 ───────

    #[test]
    fn status_401_and_403_are_classified_as_authentication_failed() {
        assert_eq!(
            parse_openrouter_response(401, br#"{"error":{}}"#).unwrap_err(),
            OpenRouterBalanceError::AuthenticationFailed(401)
        );
        assert_eq!(
            parse_openrouter_response(403, br#"{"error":{}}"#).unwrap_err(),
            OpenRouterBalanceError::AuthenticationFailed(403)
        );
        assert_eq!(
            parse_openrouter_response(500, b"boom").unwrap_err(),
            OpenRouterBalanceError::UpstreamRejected {
                status: 500,
                body: "boom".to_string(),
            }
        );
        assert_eq!(
            parse_openrouter_response(429, b"slow down").unwrap_err(),
            OpenRouterBalanceError::UpstreamRejected {
                status: 429,
                body: "slow down".to_string(),
            }
        );
    }

    #[test]
    fn upstream_rejection_keeps_the_body_for_diagnosis_but_caps_its_length() {
        // 只有 HTTP 码没法诊断:上游那句话要带上,但不能无限长。
        let error =
            parse_openrouter_response(402, br#"{"error":"insufficient credits"}"#).unwrap_err();
        assert!(
            error.to_string().contains("insufficient credits"),
            "错误串应带上游 body,实际: {error}"
        );

        let long_body = "x".repeat(OPENROUTER_UPSTREAM_BODY_MAX_CHARS * 3);
        let OpenRouterBalanceError::UpstreamRejected { body, .. } =
            parse_openrouter_response(503, long_body.as_bytes()).unwrap_err()
        else {
            panic!("503 应归为 upstream_rejected");
        };
        assert_eq!(body.chars().count(), OPENROUTER_UPSTREAM_BODY_MAX_CHARS);
    }

    #[test]
    fn error_messages_stay_in_the_same_language_as_the_other_providers() {
        // 同一个面板不能一半中文一半英文:这四条与 make_error/make_auth_error 对齐。
        for error in [
            OpenRouterBalanceError::AuthenticationFailed(401),
            OpenRouterBalanceError::ConnectionFailed("timeout".to_string()),
            OpenRouterBalanceError::InvalidResponse("bad json".to_string()),
            OpenRouterBalanceError::UpstreamRejected {
                status: 500,
                body: "boom".to_string(),
            },
        ] {
            let rendered = error.to_string();
            assert!(
                rendered.is_ascii(),
                "错误文案应与本文件其它 provider 一致(英文),实际: {rendered}"
            );
        }
    }

    #[tokio::test]
    async fn authentication_failure_writes_no_snapshot_and_keeps_the_old_one() {
        let dependencies = Arc::new(FakeOpenRouterDependencies::new());
        let old = snapshot_fixture("100.5", "25.75", "74.75", 123);
        save_snapshot(&dependencies.db, &old).unwrap();
        let old_raw = dependencies
            .db
            .get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)
            .unwrap()
            .unwrap();
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        dependencies
            .fetch_results
            .lock()
            .unwrap()
            .push_back(Err(OpenRouterBalanceError::AuthenticationFailed(401)));

        assert!(!first_failure_then_cancel(dependencies.clone()).await);
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 0);
        assert_eq!(load_snapshot(&dependencies.db).unwrap(), Some(old));
        // settings 里的旧值逐字节不动
        assert_eq!(
            dependencies
                .db
                .get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)
                .unwrap()
                .as_deref(),
            Some(old_raw.as_str())
        );
    }

    // ── 3. 网络错误 → connection_failed,旧快照保留 ───────────

    #[tokio::test]
    async fn connection_failure_keeps_the_previous_snapshot_untouched() {
        let dependencies = Arc::new(FakeOpenRouterDependencies::new());
        let old = snapshot_fixture("100.5", "25.75", "74.75", 123);
        save_snapshot(&dependencies.db, &old).unwrap();
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        dependencies.fetch_results.lock().unwrap().push_back(Err(
            OpenRouterBalanceError::ConnectionFailed("network down".to_string()),
        ));

        assert!(!first_failure_then_cancel(dependencies.clone()).await);
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 0);
        assert_eq!(load_snapshot(&dependencies.db).unwrap(), Some(old));
    }

    // 回归:改造后 DeepSeek / StepFun / SiliconFlow / Novita 四家的
    // 派发与工具函数行为不变(这四家没有既有测试,这里补最小断言)。
    #[test]
    fn balance_provider_detection_still_routes_the_original_providers() {
        assert_eq!(
            detect_provider("https://api.deepseek.com/user/balance"),
            Some(BalanceProvider::DeepSeek)
        );
        assert_eq!(
            detect_provider("https://api.stepfun.ai/v1/accounts"),
            Some(BalanceProvider::StepFun)
        );
        assert_eq!(
            detect_provider("https://api.siliconflow.cn/v1/user/info"),
            Some(BalanceProvider::SiliconFlow)
        );
        assert_eq!(
            detect_provider("https://api.siliconflow.com/v1/user/info"),
            Some(BalanceProvider::SiliconFlowEn)
        );
        assert_eq!(
            detect_provider("https://api.novita.ai/v3/user/balance"),
            Some(BalanceProvider::NovitaAI)
        );
        assert_eq!(
            detect_provider("https://openrouter.ai/api/v1/credits"),
            Some(BalanceProvider::OpenRouter)
        );
        assert_eq!(detect_provider("https://example.com"), None);
    }

    #[test]
    fn parse_f64_field_keeps_accepting_numbers_and_strings() {
        assert_eq!(parse_f64_field(&json!({"a": 1.5}), "a"), Some(1.5));
        assert_eq!(parse_f64_field(&json!({"a": "2.5"}), "a"), Some(2.5));
        assert_eq!(parse_f64_field(&json!({"a": "x"}), "a"), None);
        assert_eq!(parse_f64_field(&json!({"a": 1.5}), "missing"), None);
    }

    #[tokio::test]
    async fn get_balance_still_rejects_an_empty_key_before_any_network() {
        let result = get_balance("https://api.deepseek.com", "   ")
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("API key is empty"));
    }

    // ── 4. 缺字段/负数/非数字 → invalid_response,绝不落 0 ────

    #[test]
    fn missing_fields_are_invalid_response_not_zero() {
        let error =
            parse_openrouter_response(200, br#"{"data":{"total_credits":100.5}}"#).unwrap_err();
        assert_eq!(
            error,
            OpenRouterBalanceError::InvalidResponse(
                "Missing 'total_usage' field in response".to_string()
            )
        );
        let error = parse_openrouter_response(200, br#"{"other":{}}"#).unwrap_err();
        assert_eq!(
            error,
            OpenRouterBalanceError::InvalidResponse("Missing 'data' field in response".to_string())
        );
    }

    #[test]
    fn negative_or_non_numeric_fields_are_invalid_response() {
        let error =
            parse_openrouter_response(200, br#"{"data":{"total_credits":-1,"total_usage":0.5}}"#)
                .unwrap_err();
        assert_eq!(
            error,
            OpenRouterBalanceError::InvalidResponse("'total_credits' is negative".to_string())
        );
        let error = parse_openrouter_response(
            200,
            br#"{"data":{"total_credits":"abc","total_usage":"1"}}"#,
        )
        .unwrap_err();
        assert_eq!(
            error,
            OpenRouterBalanceError::InvalidResponse(
                "'total_credits' is not a valid number".to_string()
            )
        );
        let error =
            parse_openrouter_response(200, br#"{"data":{"total_credits":[1],"total_usage":"1"}}"#)
                .unwrap_err();
        assert_eq!(
            error,
            OpenRouterBalanceError::InvalidResponse("'total_credits' is not a number".to_string())
        );
    }

    // ── 5. 调度:15 分钟间隔 / 退避阶梯 / 取消即时退出 ────────

    #[test]
    fn refresh_interval_and_backoff_ladder_match_the_spec() {
        assert_eq!(OPENROUTER_REFRESH_INTERVAL, Duration::from_secs(15 * 60));
        assert_eq!(
            OPENROUTER_FAILURE_RETRY_DELAYS,
            [
                Duration::from_secs(30),
                Duration::from_secs(60),
                Duration::from_secs(300),
                Duration::from_secs(600),
                Duration::from_secs(1_800),
            ]
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 0),
            Duration::from_secs(30)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 1),
            Duration::from_secs(60)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 2),
            Duration::from_secs(300)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 3),
            Duration::from_secs(600)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 4),
            Duration::from_secs(1_800)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 5),
            Duration::from_secs(1_800)
        );
        assert_eq!(
            openrouter_failure_retry_delay(&OPENROUTER_FAILURE_RETRY_DELAYS, 99),
            Duration::from_secs(1_800)
        );
    }

    #[tokio::test]
    async fn successful_refresh_is_not_repeated_within_the_interval() {
        let dependencies = Arc::new(FakeOpenRouterDependencies::new());
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        let handle = start_openrouter_balance_scheduler_with_dependencies(
            dependencies.clone(),
            Duration::from_secs(1),
        );

        // 启动即第一轮,随后进入 1s 间隔(生产为 15 分钟,常量另测)
        wait_for(|| dependencies.fetch_calls.load(Ordering::SeqCst) >= 1).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 1);
        wait_for(|| dependencies.fetch_calls.load(Ordering::SeqCst) >= 2).await;
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 2);

        handle.stop().await;
    }

    #[tokio::test]
    async fn failure_backoff_retries_according_to_the_ladder_then_recovers() {
        let dependencies = FakeOpenRouterDependencies::new();
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        dependencies.fetch_results.lock().unwrap().extend([
            Err(OpenRouterBalanceError::ConnectionFailed("down".to_string())),
            Err(OpenRouterBalanceError::UpstreamRejected {
                status: 500,
                body: "boom".to_string(),
            }),
            Err(OpenRouterBalanceError::InvalidResponse("bad".to_string())),
            Ok(credits_payload("100", "20")),
        ]);
        let delays = [Duration::from_millis(10); 5];

        let started = tokio::time::Instant::now();
        refresh_with_backoff(&dependencies, &delays).await;

        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 4);
        // 三次失败各睡一档退避(下界断言,防死循环;阶梯本身由纯函数测试覆盖)
        assert!(started.elapsed() >= Duration::from_millis(30));
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 1);
        let snapshot = load_snapshot(&dependencies.db).unwrap().unwrap();
        assert_eq!(snapshot.total_credits_usd, "100");
        assert_eq!(snapshot.total_usage_usd, "20");
        assert_eq!(snapshot.balance_usd, "80");
    }

    #[tokio::test]
    async fn successful_refresh_passes_the_management_key_and_saves_then_notifies() {
        let dependencies = FakeOpenRouterDependencies::new();
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());

        refresh_with_backoff(&dependencies, &OPENROUTER_FAILURE_RETRY_DELAYS).await;
        assert_eq!(
            *dependencies.keys_seen.lock().unwrap(),
            vec!["sk-or-mgmt-test".to_string()]
        );
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 1);
        let snapshot = load_snapshot(&dependencies.db).unwrap().unwrap();
        assert_eq!(snapshot.total_credits_usd, "100.5");
        assert_eq!(snapshot.total_usage_usd, "25.75");
        assert_eq!(snapshot.balance_usd, "74.75");
        assert!(snapshot.fetched_at > 0);
    }

    #[tokio::test]
    async fn cancel_interrupts_a_pending_fetch_promptly() {
        let dependencies = Arc::new(FakeOpenRouterDependencies::new());
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        dependencies.pending_fetch.store(true, Ordering::SeqCst);
        let handle = start_openrouter_balance_scheduler_with_dependencies(
            dependencies,
            Duration::from_secs(60),
        );
        tokio::task::yield_now().await;

        tokio::time::timeout(Duration::from_millis(250), handle.stop())
            .await
            .expect("scheduler stop should not wait for the pending fetch");
    }

    #[tokio::test]
    async fn cancel_interrupts_a_failure_backoff_wait_promptly() {
        let dependencies = Arc::new(FakeOpenRouterDependencies::new());
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());
        dependencies.fetch_results.lock().unwrap().push_back(Err(
            OpenRouterBalanceError::ConnectionFailed("down".to_string()),
        ));
        let handle = start_openrouter_balance_scheduler_with_dependencies(
            dependencies.clone(),
            Duration::from_secs(60),
        );
        // 等第一次失败进入 30s 退避
        wait_for(|| dependencies.fetch_calls.load(Ordering::SeqCst) >= 1).await;

        tokio::time::timeout(Duration::from_millis(250), handle.stop())
            .await
            .expect("scheduler stop should not wait for the backoff delay");
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 1);
    }

    // ── 手动刷新:不必等调度器的 15 分钟 ─────────────────────

    #[tokio::test]
    async fn manual_refresh_without_a_management_key_fails_before_any_network() {
        let db = Database::memory().unwrap();
        let store = MemoryCredentialStore::default();

        // 没 key 就直接报错,绝不去打网络(这条测试本身也不许联网)。
        let error = manual_refresh_openrouter_balance(&db, &store)
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::InvalidInput(_)), "实际: {error}");
        assert_eq!(
            db.get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn manual_refresh_within_60_seconds_reuses_the_snapshot() {
        let db = Database::memory().unwrap();
        let store = MemoryCredentialStore::default();
        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-mgmt-test")
            .unwrap();
        let fresh = snapshot_fixture("100.5", "25.75", "74.75", unix_timestamp().unwrap());
        save_snapshot(&db, &fresh).unwrap();

        // 防连点:快照还新鲜就直接返回,不发请求(有 key 也不发)。
        manual_refresh_openrouter_balance(&db, &store)
            .await
            .unwrap();

        assert_eq!(load_snapshot(&db).unwrap().unwrap(), fresh);
    }

    #[test]
    fn manual_refresh_throttle_window_is_one_minute() {
        assert_eq!(OPENROUTER_MANUAL_REFRESH_MIN_INTERVAL_SECS, 60);
    }

    // ── 6. 无管理 key:调度空转 ───────────────────────────────

    #[tokio::test]
    async fn no_management_key_makes_the_scheduler_idle_without_fetching() {
        let dependencies = FakeOpenRouterDependencies::new();

        refresh_with_backoff(&dependencies, &OPENROUTER_FAILURE_RETRY_DELAYS).await;
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(dependencies.notifications.load(Ordering::SeqCst), 0);
        assert!(load_snapshot(&dependencies.db).unwrap().is_none());
    }

    #[tokio::test]
    async fn lightweight_mode_skips_all_work() {
        let dependencies = FakeOpenRouterDependencies::new();
        dependencies.lightweight.store(true, Ordering::SeqCst);
        dependencies
            .key
            .lock()
            .unwrap()
            .replace("sk-or-mgmt-test".to_string());

        refresh_with_backoff(&dependencies, &OPENROUTER_FAILURE_RETRY_DELAYS).await;
        assert_eq!(dependencies.fetch_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn management_key_presence_requires_some_non_empty_value() {
        let store = MemoryCredentialStore::default();
        assert!(!has_openrouter_management_key(&store));
        store.put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"").unwrap();
        assert!(!has_openrouter_management_key(&store));
        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-mgmt-test")
            .unwrap();
        assert!(has_openrouter_management_key(&store));
    }

    #[test]
    fn production_dependencies_read_the_management_key_from_the_credential_store() {
        let store = Arc::new(MemoryCredentialStore::default());
        let dependencies = ProductionOpenRouterBalanceDependencies {
            db: Arc::new(Database::memory().unwrap()),
            credentials: store.clone(),
        };
        assert_eq!(dependencies.management_key().unwrap(), None);

        store.put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"  ").unwrap();
        assert_eq!(dependencies.management_key().unwrap(), None);

        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-mgmt-test")
            .unwrap();
        assert_eq!(
            dependencies.management_key().unwrap().as_deref(),
            Some("sk-or-mgmt-test")
        );
    }

    // ── 存储层:settings 快照往返 ─────────────────────────────

    #[test]
    fn snapshot_round_trips_through_the_settings_table() {
        let db = Database::memory().unwrap();
        assert!(load_snapshot(&db).unwrap().is_none());

        let snapshot = snapshot_fixture("100.5", "25.75", "74.75", 456);
        save_snapshot(&db, &snapshot).unwrap();
        assert_eq!(load_snapshot(&db).unwrap(), Some(snapshot));

        clear_snapshot(&db).unwrap();
        assert!(load_snapshot(&db).unwrap().is_none());
        assert_eq!(
            db.get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)
                .unwrap(),
            None
        );
    }

    #[test]
    fn corrupt_snapshot_json_is_an_explicit_error_not_an_empty_view() {
        let db = Database::memory().unwrap();
        db.set_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY, "not json")
            .unwrap();
        assert!(load_snapshot(&db).is_err());
    }

    // ── 旧视图适配:不编造 0 ─────────────────────────────────

    #[test]
    fn legacy_openrouter_balance_view_keeps_its_shape_without_fabricating_zero() {
        let result = openrouter_payload_to_usage_result(credits_payload("100.5", "25.75"));
        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].remaining, Some(74.75));
        assert_eq!(data[0].total, Some(100.5));
        assert_eq!(data[0].used, Some(25.75));
        assert_eq!(data[0].is_valid, Some(true));
    }
}
