//! Codex / ChatGPT 的凭据读取与 wham 额度查询。

use serde::Deserialize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use super::{
    now_millis, unix_ts_to_iso, window_seconds_to_tier_name, CredentialStatus, ManualResetCredit,
    ManualResetCredits, QuotaTier, SubscriptionQuota,
};
use crate::credentials::codex_oauth_auth::CodexOAuthManager;
use crate::usage::system_providers::MANAGED_CODEX_QUOTA_SOURCE;

// ── Codex 凭据读取 ──────────────────────────────────────

#[derive(Deserialize)]
struct CodexAuthJson {
    auth_mode: Option<String>,
    tokens: Option<CodexTokens>,
    last_refresh: Option<String>,
}

#[derive(Deserialize)]
struct CodexTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

/// (access_token, account_id, status, message)
pub(crate) type CodexCredentials = (
    Option<String>,
    Option<String>,
    CredentialStatus,
    Option<String>,
);

/// 读取 Codex OAuth 凭据
///
/// 按优先级尝试以下来源：
/// 1. macOS Keychain (service: "Codex Auth")
/// 2. 凭据文件 ~/.codex/auth.json
///
/// 仅 auth_mode == "chatgpt" (OAuth) 时有效，API key 模式不支持用量查询。
/// T9 的 router 凭据解析(`router/auth.rs`)也要走这一个入口取 ChatGPT 的
/// access token,因此可见性提到 `pub(crate)`。
pub(crate) fn read_codex_credentials() -> CodexCredentials {
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_codex_credentials_from_keychain() {
            return result;
        }
    }

    read_codex_credentials_from_file()
}

/// 从 macOS Keychain 读取 Codex 凭据
#[cfg(target_os = "macos")]
fn read_codex_credentials_from_keychain() -> Option<CodexCredentials> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Codex Auth", "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_codex_credentials_json(json_str))
}

/// 从文件读取 Codex 凭据
fn read_codex_credentials_from_file() -> CodexCredentials {
    let auth_path = crate::agent_paths::get_codex_auth_path();

    if !auth_path.exists() {
        return (None, None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Codex auth file: {e}")),
            );
        }
    };

    parse_codex_credentials_json(&content)
}

/// 解析 Codex 凭据 JSON（Keychain 和文件共用）
fn parse_codex_credentials_json(content: &str) -> CodexCredentials {
    let auth: CodexAuthJson = match serde_json::from_str(content) {
        Ok(a) => a,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Codex auth JSON: {e}")),
            );
        }
    };

    // 仅 OAuth 模式有用量数据
    if auth.auth_mode.as_deref() != Some("chatgpt") {
        return (
            None,
            None,
            CredentialStatus::NotFound,
            Some("Codex not using OAuth mode".to_string()),
        );
    }

    let tokens = match auth.tokens {
        Some(t) => t,
        None => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("No tokens in Codex auth".to_string()),
            );
        }
    };

    let access_token = match tokens.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("access_token is empty or missing".to_string()),
            );
        }
    };

    // 检查 token 是否可能过期（距上次刷新 > 8 天）
    if let Some(ref last_refresh) = auth.last_refresh {
        if is_codex_token_stale(last_refresh) {
            return (
                Some(access_token),
                tokens.account_id,
                CredentialStatus::Expired,
                Some("Codex token may be stale (>8 days since last refresh)".to_string()),
            );
        }
    }

    (
        Some(access_token),
        tokens.account_id,
        CredentialStatus::Valid,
        None,
    )
}

/// 判断 Codex token 是否可能过期（Codex CLI 在 >8 天时自动刷新）
fn is_codex_token_stale(last_refresh: &str) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_refresh) {
        let age_secs = now_secs.saturating_sub(dt.timestamp() as u64);
        age_secs > 8 * 24 * 3600
    } else {
        false
    }
}

// ── Codex API 查询 ──────────────────────────────────────

#[derive(Deserialize)]
struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    reset_at: Option<i64>,
}

#[derive(Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexRateLimitWindow>,
    secondary_window: Option<CodexRateLimitWindow>,
}

#[derive(Deserialize)]
struct CodexAdditionalRateLimit {
    limit_name: Option<String>,
    metered_feature: Option<String>,
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Deserialize)]
struct CodexSpendControl {
    #[serde(alias = "individualLimit")]
    individual_limit: Option<CodexSpendControlLimit>,
}

#[derive(Deserialize)]
struct CodexSpendControlLimit {
    limit: Option<f64>,
    used: Option<f64>,
    #[serde(alias = "remainingPercent")]
    remaining_percent: Option<f64>,
    #[serde(alias = "resetsAt")]
    resets_at: Option<CodexResetCreditTimestamp>,
}

/// wham/usage 的 `credits`(has_credits/unlimited/balance)解析暂缓:
/// `SubscriptionQuota` 目前没有承载点数余额的字段,加了要动 30+ 处
/// 构造字面量。等有需求时随 SubscriptionQuota 扩展一并落地。
#[derive(Deserialize)]
struct CodexUsageResponse {
    rate_limit: Option<CodexRateLimit>,
    #[serde(default)]
    additional_rate_limits: Vec<CodexAdditionalRateLimit>,
    rate_limit_reset_credits: Option<CodexResetCreditSummary>,
    #[serde(default)]
    spend_control: Option<CodexSpendControl>,
}

#[derive(Deserialize)]
struct CodexResetCreditSummary {
    available_count: Option<i64>,
}

#[derive(Deserialize)]
struct CodexResetCredit {
    id: Option<String>,
    reset_type: Option<String>,
    status: Option<String>,
    granted_at: Option<CodexResetCreditTimestamp>,
    expires_at: Option<CodexResetCreditTimestamp>,
    title: Option<String>,
    description: Option<String>,
}

/// ChatGPT 的重置券详情端点目前返回 RFC 3339 字符串；兼容旧响应中的 Unix 秒。
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum CodexResetCreditTimestamp {
    Unix(i64),
    Rfc3339(String),
}

#[derive(Deserialize)]
struct CodexResetCreditsResponse {
    available_count: Option<i64>,
    #[serde(default)]
    credits: Option<Vec<CodexResetCredit>>,
}

fn codex_reset_credit_timestamp_to_iso(value: CodexResetCreditTimestamp) -> Option<String> {
    match value {
        CodexResetCreditTimestamp::Unix(timestamp) => unix_ts_to_iso(timestamp),
        CodexResetCreditTimestamp::Rfc3339(timestamp) => {
            chrono::DateTime::parse_from_rfc3339(timestamp.trim())
                .ok()
                .map(|value| value.to_rfc3339())
        }
    }
}

fn codex_additional_rate_limit_tier_name(index: usize, label: &str, window_seconds: i64) -> String {
    format!("codex_additional:{index}:{window_seconds}:{}", label.trim())
}

fn codex_usage_tiers(body: &CodexUsageResponse) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();

    let mut has_window = false;
    if let Some(rate_limit) = body.rate_limit.as_ref() {
        for window in [
            rate_limit.primary_window.as_ref(),
            rate_limit.secondary_window.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(used) = window.used_percent {
                has_window = true;
                tiers.push(QuotaTier {
                    name: window
                        .limit_window_seconds
                        .map(window_seconds_to_tier_name)
                        .unwrap_or_else(|| "unknown".to_string()),
                    utilization: used,
                    resets_at: window.reset_at.and_then(unix_ts_to_iso),
                    used_value_usd: None,
                    max_value_usd: None,
                });
            }
        }
    }

    // spend_control.individual_limit:团队/企业账户的月度额度池。
    // 只有主/周窗口都缺失时才补这个 tier,避免与窗口口径打架。
    if !has_window {
        if let Some(limit) = body
            .spend_control
            .as_ref()
            .and_then(|control| control.individual_limit.as_ref())
        {
            if let Some(utilization) = spend_control_utilization(limit) {
                tiers.push(QuotaTier {
                    name: "spend_control".to_string(),
                    utilization,
                    resets_at: limit
                        .resets_at
                        .as_ref()
                        .and_then(|value| codex_reset_credit_timestamp_to_iso(value.clone())),
                    used_value_usd: None,
                    max_value_usd: None,
                });
            }
        }
    }

    for (index, additional) in body.additional_rate_limits.iter().enumerate() {
        let label = additional
            .limit_name
            .as_deref()
            .or(additional.metered_feature.as_deref())
            .filter(|label| !label.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Additional limit {}", index + 1));
        let Some(rate_limit) = additional.rate_limit.as_ref() else {
            continue;
        };
        for window in [
            rate_limit.primary_window.as_ref(),
            rate_limit.secondary_window.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let (Some(used), Some(window_seconds)) =
                (window.used_percent, window.limit_window_seconds)
            else {
                continue;
            };
            tiers.push(QuotaTier {
                name: codex_additional_rate_limit_tier_name(index, &label, window_seconds),
                utilization: used,
                resets_at: window.reset_at.and_then(unix_ts_to_iso),
                used_value_usd: None,
                max_value_usd: None,
            });
        }
    }

    tiers
}

/// spend_control 的「已用百分比」:优先官方 remaining_percent,
/// 否则用 used/limit 推算;算不出来就整窗放弃。
fn spend_control_utilization(limit: &CodexSpendControlLimit) -> Option<f64> {
    if let Some(remaining) = limit.remaining_percent.filter(|value| value.is_finite()) {
        return Some((100.0 - remaining).clamp(0.0, 100.0));
    }
    let (Some(used), Some(max)) = (limit.used, limit.limit) else {
        return None;
    };
    if max <= 0.0 || !used.is_finite() {
        return None;
    }
    Some((used / max * 100.0).clamp(0.0, 100.0))
}

fn normalize_codex_reset_credits(
    summary_count: Option<i64>,
    response: Option<CodexResetCreditsResponse>,
) -> Option<ManualResetCredits> {
    let response_count = response.as_ref().and_then(|value| value.available_count);
    let credits = response
        .and_then(|value| value.credits)
        .unwrap_or_default()
        .into_iter()
        .filter(|credit| {
            credit
                .status
                .as_deref()
                .is_none_or(|status| status.eq_ignore_ascii_case("available"))
        })
        .filter_map(|credit| {
            let expires_at = credit
                .expires_at
                .and_then(codex_reset_credit_timestamp_to_iso)?;
            Some(ManualResetCredit {
                id: credit.id?,
                reset_type: credit.reset_type,
                status: credit.status,
                granted_at: credit
                    .granted_at
                    .and_then(codex_reset_credit_timestamp_to_iso),
                expires_at,
                title: credit.title,
                description: credit.description,
            })
        })
        .collect::<Vec<_>>();
    let available_count = summary_count
        .or(response_count)
        .unwrap_or(credits.len() as i64)
        .max(0);

    (summary_count.is_some() || response_count.is_some() || !credits.is_empty()).then_some(
        ManualResetCredits {
            available_count,
            credits,
        },
    )
}

/// 解析 ChatGPT 反代基地址:优先读 `~/.codex/config.toml` 的
/// `chatgpt_base_url`(与 CodexBar 一致),缺省用官方地址。
/// 用户常用它指向中转服务,额度接口也应当尊重该配置。
fn codex_chatgpt_base_url() -> String {
    let config_path = crate::agent_paths::get_codex_config_dir().join("config.toml");
    let raw = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|content| {
            content
                .parse::<toml::Value>()
                .ok()?
                .get("chatgpt_base_url")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| CHATGPT_OFFICIAL_BASE_URL.to_string());
    normalize_chatgpt_base_url(&raw)
}

/// 官方基地址。任何无法安全使用的配置值都回落到它。
const CHATGPT_OFFICIAL_BASE_URL: &str = "https://chatgpt.com/backend-api";

fn normalize_chatgpt_base_url(raw: &str) -> String {
    let mut base = raw.trim().to_string();
    if base.is_empty() {
        return CHATGPT_OFFICIAL_BASE_URL.to_string();
    }
    // 这个地址会收到 ChatGPT 的 OAuth access token（Bearer 头）。配置文件是
    // 不可信输入（中转商的安装脚本常改这个键），明文 http 会把 token 暴露在
    // 网络上，非 http(s) 的值更是没有意义——一律回落官方地址。
    if !base.starts_with("https://") {
        log::warn!("chatgpt_base_url 不是 https，已回落到官方地址");
        return CHATGPT_OFFICIAL_BASE_URL.to_string();
    }
    while base.ends_with('/') {
        base.pop();
    }
    if (base.starts_with("https://chatgpt.com") || base.starts_with("https://chat.openai.com"))
        && !base.contains("/backend-api")
    {
        base.push_str("/backend-api");
    }
    base
}

/// wham/usage 的完整 URL:基地址已含 /backend-api 时走官方路径,
/// 否则走 Codex 中转路径(与 CodexBar 的 URL 解析一致)。
fn codex_usage_url(base: &str) -> String {
    if base.contains("/backend-api") {
        format!("{base}/wham/usage")
    } else {
        format!("{base}/api/codex/usage")
    }
}

fn codex_reset_credits_url(base: &str) -> String {
    format!("{base}/wham/rate-limit-reset-credits")
}

/// wham 接口 429 限流冷却(进程内,对齐 claude_oauth 层)。
const CODEX_RATE_LIMIT_DEFAULT_BLOCK_SECS: u64 = 5 * 60;

fn codex_rate_limit_block() -> &'static Mutex<Option<Instant>> {
    static BLOCK: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    BLOCK.get_or_init(|| Mutex::new(None))
}

fn codex_is_rate_limited() -> bool {
    codex_rate_limit_block()
        .lock()
        .map(|guard| guard.is_some_and(|until| until > Instant::now()))
        .unwrap_or(false)
}

fn record_codex_rate_limited(retry_after_secs: Option<u64>) {
    let secs = retry_after_secs
        .unwrap_or(CODEX_RATE_LIMIT_DEFAULT_BLOCK_SECS)
        .max(1);
    if let Ok(mut guard) = codex_rate_limit_block().lock() {
        *guard = Some(Instant::now() + Duration::from_secs(secs));
    }
}

fn codex_wham_get(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    account_id: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "codex-cli")
        .header("OpenAI-Beta", "codex-1")
        .header("originator", "Codex Desktop")
        .header("Accept", "application/json");
    if let Some(id) = account_id {
        request = request.header("ChatGPT-Account-Id", id);
    }
    request.timeout(std::time::Duration::from_secs(15))
}

/// 使用 LLM Usage Bar 自管账号查询 ChatGPT 订阅额度。
///
/// 外层 manager guard 只用于克隆共享 handle；账号解析和 token 刷新由
/// `CodexOAuthManager` 的内部锁保护。因此 token 刷新与额度 HTTP 请求的
/// 网络等待都不会阻塞需要外层 write guard 的 OAuth 状态操作。
pub(crate) async fn query_managed_codex_oauth_quota(
    manager: &Arc<RwLock<CodexOAuthManager>>,
    requested_account_id: Option<&str>,
) -> Result<SubscriptionQuota, String> {
    let manager = manager.read().await.clone();
    let account_id = match requested_account_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => Some(id.to_string()),
        None => manager.default_account_id().await,
    };
    let Some(account_id) = account_id else {
        return Ok(SubscriptionQuota::not_found(MANAGED_CODEX_QUOTA_SOURCE));
    };

    let (access_token, plan_type, plan_renews_at) = match manager
        .get_valid_token_and_subscription_for_account(&account_id)
        .await
    {
        Ok(credentials) => credentials,
        Err(_) => {
            return Ok(SubscriptionQuota::error(
                MANAGED_CODEX_QUOTA_SOURCE,
                CredentialStatus::Expired,
                "Codex OAuth token unavailable. Please re-login via LLM Usage Bar.".to_string(),
            ));
        }
    };
    let mut quota = query_codex_quota(
        &access_token,
        Some(&account_id),
        MANAGED_CODEX_QUOTA_SOURCE,
        "Codex OAuth access token expired or rejected. Please re-login via LLM Usage Bar.",
    )
    .await?;
    quota.plan_type = plan_type;
    quota.plan_renews_at = plan_renews_at;
    Ok(quota)
}

/// 查询 Codex / ChatGPT 反代订阅额度。
///
/// 参数化 `tool_label` 和 `expired_message` 让 CLI 与应用自管 OAuth 路径复用
/// 同一个 wham/usage 协议实现。
pub(crate) async fn query_codex_quota(
    access_token: &str,
    account_id: Option<&str>,
    tool_label: &str,
    expired_message: &str,
) -> Result<SubscriptionQuota, String> {
    let client = crate::http_client::get();
    let base = codex_chatgpt_base_url();

    if codex_is_rate_limited() {
        // 429 冷却期内直接判失败,让回退链/调度器退避,不再打接口。
        return Err("codex_quota_rate_limited".to_string());
    }

    let resp = match codex_wham_get(&client, &codex_usage_url(&base), access_token, account_id)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Expired,
            format!("{expired_message} (HTTP {status})"),
        ));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // 记录限流冷却;后续请求在冷却期内直接失败,让链降级、不再打接口。
        let retry_after = crate::usage::claude_oauth::parse_retry_after(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            chrono::Utc::now(),
        );
        record_codex_rate_limited(retry_after);
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Valid,
            "codex_quota_rate_limited".to_string(),
        ));
    }

    if !status.is_success() {
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Valid,
            format!("API error (HTTP {status})"),
        ));
    }

    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read API response: {e}")),
    };
    let body: CodexUsageResponse = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                tool_label,
                CredentialStatus::Valid,
                format!("Failed to parse API response: {e}"),
            ));
        }
    };

    let reset_credit_count = body
        .rate_limit_reset_credits
        .as_ref()
        .and_then(|credits| credits.available_count);
    let should_fetch_reset_credit_details = reset_credit_count.is_some_and(|count| count > 0);
    let tiers = codex_usage_tiers(&body);
    let reset_credit_response = if should_fetch_reset_credit_details {
        match codex_wham_get(
            &client,
            &codex_reset_credits_url(&base),
            access_token,
            account_id,
        )
        .send()
        .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<CodexResetCreditsResponse>().await {
                    Ok(value) => Some(value),
                    Err(error) => {
                        log::warn!("failed to parse Codex reset credit details: {error}");
                        None
                    }
                }
            }
            Ok(response) => {
                log::warn!(
                    "failed to fetch Codex reset credit details: HTTP {}",
                    response.status()
                );
                None
            }
            Err(error) => {
                log::warn!("failed to fetch Codex reset credit details: {error}");
                None
            }
        }
    } else {
        None
    };
    let manual_reset_credits =
        normalize_codex_reset_credits(reset_credit_count, reset_credit_response);

    Ok(SubscriptionQuota {
        success: true,
        tiers,
        manual_reset_credits,
        queried_at: Some(now_millis()),
        ..SubscriptionQuota::skeleton(tool_label)
    })
}

// 说明:这里刻意**不**用 Codex CLI 的 refresh_token 去 `auth.openai.com/oauth/token`
// 续期。OAuth refresh token 通常是轮换式的 —— 用一次就作废旧的,而服务端返回的
// 新 token 需要写回 `~/.codex/auth.json`。本 app 是只读监控方,不该成为 auth.json
// 的写入方;而"刷了却不回写"会让 Codex CLI 自己存的 refresh_token 变成陈旧值,
// 把用户从 Codex CLI 里踢下线 —— 监控工具把被监控工具搞挂,收益不抵风险。
// 凭据过期时直接报 Expired,由回退链的下一层(CLI 探测)出数,重新登录交给用户
// 在 Codex CLI 侧完成。

/// Codex 的订阅额度采集：读 CLI 凭据 → 按凭据状态分流 → 查 wham 接口。
///
/// 与 Gemini 的差别在 `Expired`：Codex 不做 token 续期（refresh token 是轮换式的，
/// 本 app 只读不写 `auth.json`，刷了不回写会把用户从 Codex CLI 踢下线），过期就
/// 拿旧 token 试一把，不成就如实报过期。
pub(super) async fn collect_codex_quota() -> Result<SubscriptionQuota, String> {
    const TOOL: &str = "codex";
    const EXPIRED_MESSAGE: &str = "Authentication failed. Please re-login with Codex CLI.";
    let (token, account_id, status, message) = read_codex_credentials();

    match status {
        CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found(TOOL)),
        CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
            TOOL,
            CredentialStatus::ParseError,
            message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
        )),
        CredentialStatus::Expired => {
            // 即使可能过期也尝试调用 API
            if let Some(token) = token {
                let result =
                    query_codex_quota(&token, account_id.as_deref(), TOOL, EXPIRED_MESSAGE).await?;
                if result.success {
                    return Ok(result);
                }
            }
            Ok(SubscriptionQuota::error(
                TOOL,
                CredentialStatus::Expired,
                message.unwrap_or_else(|| "Codex OAuth token may be stale".to_string()),
            ))
        }
        CredentialStatus::Valid => {
            let token = token.expect("token must be Some when status is Valid");
            query_codex_quota(&token, account_id.as_deref(), TOOL, EXPIRED_MESSAGE).await
        }
        _ => Ok(SubscriptionQuota::not_found(TOOL)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 常量留在父模块 mod.rs，`use super::*` 拿不到，这里补一条。
    use crate::services::subscription::TIER_SEVEN_DAY;

    #[test]
    fn chatgpt_base_url_falls_back_unless_https() {
        // 明文 http / 非 http(s) / 空值都不允许携带 Bearer token 出去。
        assert_eq!(
            normalize_chatgpt_base_url("http://relay.example.com"),
            CHATGPT_OFFICIAL_BASE_URL
        );
        assert_eq!(
            normalize_chatgpt_base_url("relay.example.com"),
            CHATGPT_OFFICIAL_BASE_URL
        );
        assert_eq!(
            normalize_chatgpt_base_url("file:///etc/passwd"),
            CHATGPT_OFFICIAL_BASE_URL
        );
        assert_eq!(normalize_chatgpt_base_url("   "), CHATGPT_OFFICIAL_BASE_URL);
    }

    #[test]
    fn chatgpt_base_url_keeps_https_values_and_official_suffix() {
        // 官方裸域补 /backend-api；已带后缀的不重复补。
        assert_eq!(
            normalize_chatgpt_base_url("https://chatgpt.com"),
            CHATGPT_OFFICIAL_BASE_URL
        );
        assert_eq!(
            normalize_chatgpt_base_url("https://chatgpt.com/backend-api/"),
            CHATGPT_OFFICIAL_BASE_URL
        );
        // https 的第三方中转按原样保留（用户自己的配置）。
        assert_eq!(
            normalize_chatgpt_base_url("https://relay.example.com/v1/"),
            "https://relay.example.com/v1"
        );
    }

    #[test]
    fn codex_usage_url_switches_on_backend_api_suffix() {
        assert_eq!(
            codex_usage_url(CHATGPT_OFFICIAL_BASE_URL),
            "https://chatgpt.com/backend-api/wham/usage"
        );
        assert_eq!(
            codex_usage_url("https://relay.example.com/v1"),
            "https://relay.example.com/v1/api/codex/usage"
        );
    }

    #[test]
    fn codex_usage_keeps_named_additional_rate_limit_windows() {
        let body: CodexUsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {
                "secondary_window": {
                    "used_percent": 4,
                    "limit_window_seconds": 604800,
                    "reset_at": 1_800_000_000
                }
            },
            "additional_rate_limits": [
                {
                    "limit_name": "Codex Spark",
                    "rate_limit": {
                        "primary_window": {
                            "used_percent": 10,
                            "limit_window_seconds": 18000,
                            "reset_at": 1_800_003_600
                        },
                        "secondary_window": {
                            "used_percent": 20,
                            "limit_window_seconds": 604800,
                            "reset_at": 1_800_604_800
                        }
                    }
                }
            ]
        }))
        .unwrap();

        let tiers = codex_usage_tiers(&body);

        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, TIER_SEVEN_DAY);
        assert_eq!(tiers[1].name, "codex_additional:0:18000:Codex Spark");
        assert_eq!(tiers[2].name, "codex_additional:0:604800:Codex Spark");
        assert!(tiers[1].resets_at.is_some());
        assert!(tiers[2].resets_at.is_some());
    }

    #[test]
    fn codex_manual_reset_credits_keep_available_items_and_authoritative_count() {
        let response: CodexResetCreditsResponse = serde_json::from_value(serde_json::json!({
            "available_count": 4,
            "credits": [
                {
                    "id": "reset-1",
                    "reset_type": "codexRateLimits",
                    "status": "available",
                    "granted_at": "2026-06-26T23:44:25Z",
                    "expires_at": "2026-07-26T23:44:25Z",
                    "title": "Full reset"
                },
                {
                    "id": "reset-2",
                    "status": "consumed",
                    "expires_at": "2026-07-31T19:03:39Z",
                    "title": "Full reset"
                },
                {
                    "id": "reset-3",
                    "status": "available",
                    "expires_at": 1786555942,
                    "title": "Full reset"
                },
                {
                    "id": "missing-expiry",
                    "status": "available"
                }
            ]
        }))
        .unwrap();

        let credits = normalize_codex_reset_credits(Some(3), Some(response)).unwrap();

        assert_eq!(credits.available_count, 3);
        assert_eq!(credits.credits.len(), 2);
        assert_eq!(credits.credits[0].title.as_deref(), Some("Full reset"));
        assert!(credits.credits[0].expires_at.starts_with("2026-07-26"));
        assert!(credits.credits[1].expires_at.starts_with("2026-08-12"));

        let count_only = normalize_codex_reset_credits(
            Some(2),
            Some(CodexResetCreditsResponse {
                available_count: Some(2),
                credits: None,
            }),
        )
        .unwrap();
        assert_eq!(count_only.available_count, 2);
        assert!(count_only.credits.is_empty());
    }
}
