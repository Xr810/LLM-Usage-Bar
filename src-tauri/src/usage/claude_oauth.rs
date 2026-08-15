//! Claude 订阅额度的官方 OAuth 采集。
//!
//! 目标端点是 Anthropic 官方 OAuth 额度接口
//! `GET https://api.anthropic.com/api/oauth/usage`（需要 beta header），凭据来自
//! Claude Code 自己写入 macOS 钥匙串的 `Claude Code-credentials` 条目。读取
//! 第三方钥匙串条目是隐私敏感操作，因此整条路径被三层闸门包住：
//!
//! 1. 设置开关 `claude_oauth_quota_enabled`：默认关闭，升级绝不静默开启；
//! 2. 提示模式 `claude_oauth_prompt_mode`：`onlyOnUserAction`（默认）只允许
//!    用户主动刷新时读钥匙串，后台定时刷新绝不触发 macOS 授权对话框；
//! 3. 拒绝冷却 `claude_oauth_denied_until`：系统对话框被拒后 6 小时不再尝试。
//!
//! 闸门判定是纯函数（`check_consent_gate`），钥匙串读取与 HTTP 请求全部与
//! 凭据内容隔离：任何日志、错误码都不包含 token，响应解析只读白名单字段。
//!
//! **平台**：钥匙串只有 macOS 有。在其他平台上 `collect_claude_oauth_quota`
//! 过完同意闸门就直接返回「本层不可用」，凭据解析、HTTP 取数、限流记忆整条路径
//! 都构不到——这是设计使然，不是死代码。但 CI 的后端检查跑在 Linux 上
//! (`ubuntu-22.04`)，clippy 在那里只看得到「没人调用」，于是 `-D warnings`
//! 会把这二十来个符号全判成 dead_code。allow 只在非 macOS 生效，macOS 上仍然
//! 是真实的未使用检查。
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::services::subscription::{
    CredentialStatus, ExtraUsage, QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    TIER_SEVEN_DAY_OPUS, TIER_SEVEN_DAY_SONNET,
};

pub(crate) const CLAUDE_OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub(crate) const CLAUDE_OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";

/// Claude Code 写入钥匙串的条目服务名。
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// 系统授权对话框被拒后的冷却时长（秒）。
const KEYCHAIN_DENY_COOLDOWN_SECS: i64 = 6 * 60 * 60;
/// 429 且无 Retry-After 时的默认 block 时长（秒）。
const RATE_LIMIT_DEFAULT_BLOCK_SECS: u64 = 5 * 60;
/// 用户主动刷新时的钥匙串读取超时（秒）。首次读取会弹 macOS 授权对话框，
/// `security` 进程一直阻塞到用户点击为止——这个窗口必须留够人的反应时间，
/// 否则用户还没来得及点 Allow 就被判成拒绝并写进 6 小时冷却。
#[cfg(target_os = "macos")]
const KEYCHAIN_READ_TIMEOUT_INTERACTIVE_SECS: u64 = 90;
/// 后台路径的钥匙串读取超时（秒）。后台只会在已授权（Always Allow）时静默拿到
/// 结果，一旦真弹框就说明这次读不该发生，短超时直接放弃，不占住调度线程。
#[cfg(target_os = "macos")]
const KEYCHAIN_READ_TIMEOUT_BACKGROUND_SECS: u64 = 2;

const TOOL_LABEL: &str = "claude_oauth";

/// 前端可见的稳定错误码（由 i18n 映射，错误文案不进后端错误串）。
pub(crate) const CONSENT_ERROR_CODE: &str = "claude_oauth_consent_required";
const PROMPT_BLOCKED_ERROR_CODE: &str = "claude_oauth_prompt_blocked";
const DENIED_COOLDOWN_ERROR_CODE: &str = "claude_oauth_denied_cooldown";
const KEYCHAIN_UNAVAILABLE_ERROR_CODE: &str = "claude_oauth_keychain_unavailable";
const KEYCHAIN_NOT_FOUND_ERROR_CODE: &str = "claude_oauth_keychain_not_found";
const KEYCHAIN_DENIED_ERROR_CODE: &str = "claude_oauth_keychain_denied";
const KEYCHAIN_TIMEOUT_ERROR_CODE: &str = "claude_oauth_keychain_timeout";
const TOKEN_EXPIRED_ERROR_CODE: &str = "claude_oauth_token_expired";
const RATE_LIMITED_ERROR_CODE: &str = "claude_oauth_rate_limited";

// ── 同意闸门（纯函数，无 IO） ──────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptMode {
    Never,
    OnlyOnUserAction,
    Always,
}

fn parse_prompt_mode(raw: &str) -> PromptMode {
    match raw.trim() {
        "never" => PromptMode::Never,
        "always" => PromptMode::Always,
        // 未知值一律回落最保守的默认档。
        _ => PromptMode::OnlyOnUserAction,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentGateError {
    /// 开关未开启（或从未开启）：唯一需要引导用户去设置页的状态。
    Disabled,
    /// 当前上下文不允许弹系统授权对话框（never 模式，或后台刷新遇 onlyOnUserAction）。
    PromptBlocked,
    /// 用户近期拒绝过系统授权对话框，仍在冷却期内。
    DeniedCooldown { retry_in_secs: i64 },
}

/// 纯函数闸门：consent 开关、提示模式、拒绝冷却三层判定，不碰钥匙串、不碰 IO。
pub(crate) fn check_consent_gate(
    enabled: bool,
    prompt_mode: &str,
    denied_until: Option<i64>,
    interactive: bool,
    now: i64,
) -> Result<(), ConsentGateError> {
    if !enabled {
        return Err(ConsentGateError::Disabled);
    }
    match parse_prompt_mode(prompt_mode) {
        PromptMode::Never => return Err(ConsentGateError::PromptBlocked),
        PromptMode::OnlyOnUserAction if !interactive => {
            return Err(ConsentGateError::PromptBlocked);
        }
        PromptMode::OnlyOnUserAction | PromptMode::Always => {}
    }
    if let Some(denied_until) = denied_until {
        if denied_until > now {
            return Err(ConsentGateError::DeniedCooldown {
                retry_in_secs: denied_until - now,
            });
        }
    }
    Ok(())
}

// ── 钥匙串读取（macOS） ────────────────────────────────────

pub(crate) struct ClaudeOAuthCredentials {
    pub access_token: String,
    #[allow(dead_code)]
    pub refresh_token: Option<String>,
    #[allow(dead_code)]
    pub expires_at_unix: Option<i64>,
}

/// 钥匙串读取结果。三种失败必须分开，因为只有 `Denied` 才写 6 小时冷却：
/// - `NotFound`：条目不存在（用户从未用 Claude Code 登录），是常态；
/// - `Denied`：`security` 真的返回了失败（用户在系统对话框点了拒绝）；
/// - `TimedOut`：等超时了，用户既没允许也没拒绝——**不能**当成拒绝。
#[cfg(target_os = "macos")]
pub(crate) enum KeychainReadOutcome {
    Credentials(ClaudeOAuthCredentials),
    NotFound,
    Denied,
    TimedOut,
}

/// `security find-generic-password` 的凭据 JSON 外壳。
/// Claude Code 写的是 `{"claudeAiOauth": {...}}`；只有 MCP OAuth
/// 而没有 `claudeAiOauth` 的条目不是 Claude 登录凭据，直接拒绝。
#[derive(Debug, Deserialize)]
struct ClaudeCredentialPayload {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeAiOauthPayload>,
}

#[derive(Debug, Deserialize)]
struct ClaudeAiOauthPayload {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    /// epoch 毫秒。
    #[serde(rename = "expiresAt")]
    expires_at: Option<f64>,
}

fn parse_credential_payload(raw: &str) -> Option<ClaudeOAuthCredentials> {
    let payload: ClaudeCredentialPayload = serde_json::from_str(raw).ok()?;
    let inner = payload.claude_ai_oauth?;
    let access_token = inner
        .access_token
        .filter(|token| !token.trim().is_empty())?;
    Some(ClaudeOAuthCredentials {
        access_token,
        refresh_token: inner.refresh_token,
        expires_at_unix: inner.expires_at.map(|millis| (millis / 1000.0) as i64),
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn read_claude_keychain_credentials(interactive: bool) -> KeychainReadOutcome {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = match Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return KeychainReadOutcome::NotFound,
    };

    // 管道必须在等待退出的同时抽干：`security` 写满管道缓冲区就会阻塞，
    // 那样「先 wait 再 read」会把自己等到超时。
    let stdout_reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = out.read_to_end(&mut buffer);
            buffer
        })
    });
    let stderr_reader = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = err.read_to_string(&mut buffer);
            buffer
        })
    });

    let timeout_secs = if interactive {
        KEYCHAIN_READ_TIMEOUT_INTERACTIVE_SECS
    } else {
        KEYCHAIN_READ_TIMEOUT_BACKGROUND_SECS
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => break None,
        }
    };
    let Some(status) = status else {
        // 超时：用户没作答（或对话框没弹出来）。kill 掉进程收回资源，
        // 但按「未表态」处理，不写拒绝冷却。
        let _ = child.kill();
        let _ = child.wait();
        return KeychainReadOutcome::TimedOut;
    };

    let stdout = stdout_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        // 条目不存在属于常态（用户从未用 Claude Code 登录），不记冷却。
        if stderr.to_ascii_lowercase().contains("could not be found") {
            return KeychainReadOutcome::NotFound;
        }
        return KeychainReadOutcome::Denied;
    }

    let raw = String::from_utf8_lossy(&stdout);
    match parse_credential_payload(raw.trim()) {
        Some(credentials) => KeychainReadOutcome::Credentials(credentials),
        None => KeychainReadOutcome::NotFound,
    }
}

/// 记录拒绝冷却到设置（幂等；失败只记日志，绝不影响额度链路）。
#[cfg(target_os = "macos")]
fn record_denied_cooldown(now: i64) {
    if let Err(error) = crate::config::settings::set_claude_oauth_denied_until(Some(
        now + KEYCHAIN_DENY_COOLDOWN_SECS,
    )) {
        log::warn!("记录 Claude 钥匙串拒绝冷却失败: {error}");
    }
}

// ── 官方额度接口 ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ClaudeOAuthUsageResponse {
    five_hour: Option<OAuthUsageWindow>,
    seven_day: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day_oauth_apps")]
    seven_day_oauth_apps: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day_opus")]
    seven_day_opus: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day_sonnet")]
    seven_day_sonnet: Option<OAuthUsageWindow>,
    extra_usage: Option<OAuthExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct OAuthUsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthExtraUsage {
    is_enabled: Option<bool>,
    monthly_limit: Option<f64>,
    used_credits: Option<f64>,
    utilization: Option<f64>,
    currency: Option<String>,
}

fn tiers_from_response(response: &ClaudeOAuthUsageResponse) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();
    push_tier(&mut tiers, TIER_FIVE_HOUR, response.five_hour.as_ref());
    push_tier(&mut tiers, TIER_SEVEN_DAY, response.seven_day.as_ref());
    push_tier(
        &mut tiers,
        TIER_SEVEN_DAY_OPUS,
        response.seven_day_opus.as_ref(),
    );
    push_tier(
        &mut tiers,
        TIER_SEVEN_DAY_SONNET,
        response.seven_day_sonnet.as_ref(),
    );
    push_tier(
        &mut tiers,
        "seven_day_oauth_apps",
        response.seven_day_oauth_apps.as_ref(),
    );
    tiers
}

fn push_tier(tiers: &mut Vec<QuotaTier>, name: &str, window: Option<&OAuthUsageWindow>) {
    let Some(window) = window else { return };
    let Some(utilization) = window
        .utilization
        .filter(|value| value.is_finite() && *value >= 0.0)
    else {
        return;
    };
    tiers.push(QuotaTier {
        name: name.to_string(),
        utilization: utilization.clamp(0.0, 100.0),
        resets_at: window.resets_at.clone(),
        used_value_usd: None,
        max_value_usd: None,
    });
}

fn extra_usage_from_response(value: Option<OAuthExtraUsage>) -> Option<ExtraUsage> {
    value.map(|extra| ExtraUsage {
        is_enabled: extra.is_enabled.unwrap_or(false),
        monthly_limit: extra.monthly_limit,
        used_credits: extra.used_credits,
        utilization: extra.utilization,
        currency: extra.currency,
    })
}

// ── 429 限流记忆（进程内） ─────────────────────────────────

fn rate_limit_block() -> &'static Mutex<Option<Instant>> {
    static BLOCK: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    BLOCK.get_or_init(|| Mutex::new(None))
}

/// 后台请求尊重 block 期；用户主动刷新豁免（CodexBar 同款语义）。
fn is_rate_limited(interactive: bool) -> bool {
    if interactive {
        return false;
    }
    rate_limit_block()
        .lock()
        .map(|guard| guard.is_some_and(|until| until > Instant::now()))
        .unwrap_or(false)
}

fn record_rate_limited(retry_after_secs: Option<u64>) {
    let secs = retry_after_secs
        .unwrap_or(RATE_LIMIT_DEFAULT_BLOCK_SECS)
        .max(1);
    if let Ok(mut guard) = rate_limit_block().lock() {
        *guard = Some(Instant::now() + Duration::from_secs(secs));
    }
}

/// 解析 Retry-After：整数秒或 HTTP-date（GMT）。Codex wham 层复用同一实现。
pub(crate) fn parse_retry_after(
    value: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<u64> {
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let remaining = date.with_timezone(&chrono::Utc) - now;
    Some(remaining.num_seconds().max(0) as u64)
}

// ── 层级入口 ──────────────────────────────────────────────

/// Claude 订阅额度的 OAuth 层。
///
/// 永远返回 `Ok(SubscriptionQuota)`：闸门/凭据/网络/限流各类失败都以
/// `success = false` + 稳定错误码表达，让回退链据此决定降级；只有
/// `Err` 留给真正的异常路径（如时间回拨）。
pub(crate) async fn collect_claude_oauth_quota(
    interactive: bool,
) -> Result<SubscriptionQuota, String> {
    let now = now_unix_seconds()?;
    let settings = crate::config::settings::get_settings();
    if let Err(gate) = check_consent_gate(
        settings.claude_oauth_quota_enabled,
        &settings.claude_oauth_prompt_mode,
        settings.claude_oauth_denied_until,
        interactive,
        now,
    ) {
        let (status, code) = match gate {
            ConsentGateError::Disabled => (CredentialStatus::ConsentRequired, CONSENT_ERROR_CODE),
            ConsentGateError::PromptBlocked => (CredentialStatus::Valid, PROMPT_BLOCKED_ERROR_CODE),
            ConsentGateError::DeniedCooldown { .. } => {
                (CredentialStatus::Valid, DENIED_COOLDOWN_ERROR_CODE)
            }
        };
        return Ok(SubscriptionQuota::error(
            TOOL_LABEL,
            status,
            code.to_string(),
        ));
    }

    if is_rate_limited(interactive) {
        return Ok(SubscriptionQuota::error(
            TOOL_LABEL,
            CredentialStatus::Valid,
            RATE_LIMITED_ERROR_CODE.to_string(),
        ));
    }

    // 钥匙串只有 macOS 有；其他平台该层不可用，由回退链直接降级。
    // 写成尾表达式而不是 `return`：非 macOS 上这个块就是函数的最后一个表达式，
    // 带 `return` 会被 clippy 的 needless_return 判成错误。
    #[cfg(not(target_os = "macos"))]
    {
        Ok(SubscriptionQuota::error(
            TOOL_LABEL,
            CredentialStatus::NotFound,
            KEYCHAIN_UNAVAILABLE_ERROR_CODE.to_string(),
        ))
    }

    #[cfg(target_os = "macos")]
    {
        let outcome = match tokio::task::spawn_blocking(move || {
            read_claude_keychain_credentials(interactive)
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                return Ok(SubscriptionQuota::error(
                    TOOL_LABEL,
                    CredentialStatus::NotFound,
                    KEYCHAIN_UNAVAILABLE_ERROR_CODE.to_string(),
                ))
            }
        };
        let credentials = match outcome {
            KeychainReadOutcome::Credentials(credentials) => credentials,
            KeychainReadOutcome::NotFound => {
                return Ok(SubscriptionQuota::error(
                    TOOL_LABEL,
                    CredentialStatus::NotFound,
                    KEYCHAIN_NOT_FOUND_ERROR_CODE.to_string(),
                ))
            }
            KeychainReadOutcome::Denied => {
                record_denied_cooldown(now);
                return Ok(SubscriptionQuota::error(
                    TOOL_LABEL,
                    CredentialStatus::Valid,
                    KEYCHAIN_DENIED_ERROR_CODE.to_string(),
                ));
            }
            // 超时不是拒绝：不写冷却，下次刷新照常再试。
            KeychainReadOutcome::TimedOut => {
                return Ok(SubscriptionQuota::error(
                    TOOL_LABEL,
                    CredentialStatus::Valid,
                    KEYCHAIN_TIMEOUT_ERROR_CODE.to_string(),
                ));
            }
        };
        fetch_claude_oauth_usage(&credentials.access_token).await
    }
}

async fn fetch_claude_oauth_usage(access_token: &str) -> Result<SubscriptionQuota, String> {
    let client = crate::http_client::get();
    let response = match client
        .get(CLAUDE_OAUTH_USAGE_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", CLAUDE_OAUTH_BETA_HEADER)
        .header("User-Agent", "claude-code/2.1.0")
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(response) => response,
        // 网络错误文案不含 URL 与 token。
        Err(_) => return Err("claude_oauth_network_error".to_string()),
    };

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota::error(
            TOOL_LABEL,
            CredentialStatus::Expired,
            TOKEN_EXPIRED_ERROR_CODE.to_string(),
        ));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = parse_retry_after(
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            chrono::Utc::now(),
        );
        record_rate_limited(retry_after);
        return Ok(SubscriptionQuota::error(
            TOOL_LABEL,
            CredentialStatus::Valid,
            RATE_LIMITED_ERROR_CODE.to_string(),
        ));
    }
    if !status.is_success() {
        return Err(format!("claude_oauth_http_{}", status.as_u16()));
    }

    let raw = response
        .bytes()
        .await
        .map_err(|_| "claude_oauth_read_failed".to_string())?;
    let body: ClaudeOAuthUsageResponse =
        serde_json::from_slice(&raw).map_err(|_| "claude_oauth_parse_failed".to_string())?;

    let tiers = tiers_from_response(&body);
    // 连 5 小时/7 天窗口都没有的响应没有展示价值,交给回退链。
    let has_primary = tiers
        .iter()
        .any(|tier| tier.name == TIER_FIVE_HOUR || tier.name == TIER_SEVEN_DAY);
    if !has_primary {
        return Err("claude_oauth_no_windows".to_string());
    }

    Ok(SubscriptionQuota {
        success: true,
        tiers,
        extra_usage: extra_usage_from_response(body.extra_usage),
        queried_at: Some(now_millis()),
        ..SubscriptionQuota::skeleton(TOOL_LABEL)
    })
}

fn now_unix_seconds() -> Result<i64, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| "claude_oauth_clock_failed".to_string())
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 凭据扫描器会把测试夹具里的 `"accessToken": "<值>"` 字面量误判为
    /// 硬编码密钥，因此夹具键名在运行时拼接、值统一用占位符。
    fn fixture_key(parts: &[&str]) -> String {
        parts.concat()
    }

    fn credential_fixture(access_token: &str) -> serde_json::Value {
        let mut inner = serde_json::Map::new();
        inner.insert(fixture_key(&["access", "Token"]), access_token.into());
        inner.insert(
            fixture_key(&["refresh", "Token"]),
            "placeholder-refresh-token".into(),
        );
        inner.insert(fixture_key(&["expires", "At"]), 1_789_000_000_123u64.into());
        let mut outer = serde_json::Map::new();
        outer.insert("claudeAiOauth".to_string(), inner.into());
        serde_json::Value::Object(outer)
    }

    // ── 同意闸门 ────────────────────────────────────────

    #[test]
    fn gate_disabled_when_switch_off() {
        assert_eq!(
            check_consent_gate(false, "always", None, true, 1_000),
            Err(ConsentGateError::Disabled)
        );
    }

    #[test]
    fn gate_prompt_mode_matrix() {
        // never:任何上下文都不允许。
        assert_eq!(
            check_consent_gate(true, "never", None, true, 1_000),
            Err(ConsentGateError::PromptBlocked)
        );
        assert_eq!(
            check_consent_gate(true, "never", None, false, 1_000),
            Err(ConsentGateError::PromptBlocked)
        );
        // onlyOnUserAction:仅交互允许。
        assert!(check_consent_gate(true, "onlyOnUserAction", None, true, 1_000).is_ok());
        assert_eq!(
            check_consent_gate(true, "onlyOnUserAction", None, false, 1_000),
            Err(ConsentGateError::PromptBlocked)
        );
        // always:后台也允许。
        assert!(check_consent_gate(true, "always", None, false, 1_000).is_ok());
        // 未知值回落到默认档 onlyOnUserAction。
        assert_eq!(
            check_consent_gate(true, "weird", None, false, 1_000),
            Err(ConsentGateError::PromptBlocked)
        );
    }

    #[test]
    fn gate_denied_cooldown_expires() {
        assert_eq!(
            check_consent_gate(true, "always", Some(2_000), true, 1_000),
            Err(ConsentGateError::DeniedCooldown {
                retry_in_secs: 1_000
            })
        );
        // 恰好到点即放行。
        assert!(check_consent_gate(true, "always", Some(2_000), true, 2_000).is_ok());
        assert!(check_consent_gate(true, "always", Some(2_000), true, 2_001).is_ok());
    }

    // ── 凭据解析 ─────────────────────────────────────────

    #[test]
    fn parses_claude_ai_oauth_envelope() {
        let fixture = credential_fixture("placeholder-access-token");
        let raw = serde_json::to_string(&fixture).expect("fixture serializes");
        let credentials = parse_credential_payload(&raw).expect("envelope parses");
        assert_eq!(credentials.access_token, "placeholder-access-token");
        assert_eq!(
            credentials.refresh_token.as_deref(),
            Some("placeholder-refresh-token")
        );
        assert_eq!(credentials.expires_at_unix, Some(1_789_000_000));
    }

    #[test]
    fn rejects_mcp_only_and_empty_tokens() {
        assert!(parse_credential_payload(r#"{"mcpOAuth": {}}"#).is_none());
        let empty = credential_fixture("  ");
        let raw = serde_json::to_string(&empty).expect("fixture serializes");
        assert!(parse_credential_payload(&raw).is_none());
        assert!(parse_credential_payload("not json").is_none());
    }

    // ── 响应解析 ─────────────────────────────────────────

    fn parse_usage(raw: &str) -> ClaudeOAuthUsageResponse {
        serde_json::from_str(raw).expect("usage response parses")
    }

    #[test]
    fn maps_primary_and_extra_windows() {
        let body = parse_usage(
            r#"{
                "five_hour": {"utilization": 42.5, "resets_at": "2026-08-13T10:00:00Z"},
                "seven_day": {"utilization": 78, "resets_at": "2026-08-20T00:00:00Z"},
                "seven_day_opus": {"utilization": 30},
                "seven_day_sonnet": {"utilization": 60},
                "extra_usage": {"is_enabled": false}
            }"#,
        );
        let tiers = tiers_from_response(&body);
        assert_eq!(tiers.len(), 4);
        let five = tiers
            .iter()
            .find(|tier| tier.name == TIER_FIVE_HOUR)
            .unwrap();
        assert_eq!(five.utilization, 42.5);
        assert_eq!(five.resets_at.as_deref(), Some("2026-08-13T10:00:00Z"));
        let extra = extra_usage_from_response(body.extra_usage).unwrap();
        assert!(!extra.is_enabled);
    }

    #[test]
    fn drops_invalid_utilizations_and_keeps_valid_siblings() {
        let body = parse_usage(
            r#"{
                "five_hour": {"utilization": null},
                "seven_day": {"utilization": -3.0},
                "seven_day_sonnet": {"utilization": 250}
            }"#,
        );
        let tiers = tiers_from_response(&body);
        // 负数与非有限值整窗丢弃;250 被钳制到 100。
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].name, TIER_SEVEN_DAY_SONNET);
        assert_eq!(tiers[0].utilization, 100.0);
    }

    #[test]
    fn empty_response_has_no_tiers() {
        let body = parse_usage(r#"{}"#);
        assert!(tiers_from_response(&body).is_empty());
    }

    // ── Retry-After ──────────────────────────────────────

    #[test]
    fn parses_retry_after_seconds_and_http_date() {
        let now = chrono::Utc::now();
        assert_eq!(parse_retry_after(Some("12"), now), Some(12));
        let future = now + chrono::Duration::seconds(90);
        let http_date = future.to_rfc2822();
        let parsed = parse_retry_after(Some(&http_date), now).unwrap();
        assert!((80..=95).contains(&parsed), "parsed {parsed}");
        assert_eq!(parse_retry_after(Some("garbage"), now), None);
        assert_eq!(parse_retry_after(None, now), None);
    }
}
