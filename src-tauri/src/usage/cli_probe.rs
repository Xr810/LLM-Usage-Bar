//! Claude Code CLI 用量探测：回退链的第二层。
//!
//! 在真终端（PTY）里拉起 `claude`，向 TUI 发送 `/usage`，解析渲染出来的
//! 用量百分比（"Current session" / "Current week (all models)" 及 Opus/Sonnet
//! 窗口）。与 CodexBar 的 ClaudeStatusProbe 机制一致，但只保留额度百分比
//! 这一条窄链路。
//!
//! 探测很重（拉起一个交互式 CLI 进程），因此：
//! - 调度器（后台）路径上两次探测之间至少间隔 30 分钟；
//! - 用户手动刷新豁免最小间隔，但仍受失败退避约束；
//! - 任何失败只产生稳定错误码，绝不含终端原文（可能带敏感信息）。
//!
//! 文本解析是纯函数（`parse_claude_usage_text`），测试用夹具文本覆盖，
//! 不真实拉起进程。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::subscription::{
    CredentialStatus, QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    TIER_SEVEN_DAY_OPUS, TIER_SEVEN_DAY_SONNET,
};

const TOOL_LABEL: &str = "claude_probe";

/// 探测总超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// 调度器（非交互）路径上的最小探测间隔。
const BACKGROUND_MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// 探测失败后的退避（对交互与后台统一生效，避免反复拉起失败的 CLI）。
const FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);

/// 错误码（稳定、前端可映射；不带终端原文）。
const THROTTLED_ERROR_CODE: &str = "claude_probe_throttled";
const BACKOFF_ERROR_CODE: &str = "claude_probe_backoff";
const NOT_INSTALLED_ERROR_CODE: &str = "claude_probe_not_installed";
const FAILED_ERROR_CODE: &str = "claude_probe_failed";

// ── 探测闸门 ──────────────────────────────────────────────

struct ProbeGate {
    last_attempt: Mutex<Option<Instant>>,
    last_failure: Mutex<Option<Instant>>,
}

impl ProbeGate {
    fn new() -> Self {
        Self {
            last_attempt: Mutex::new(None),
            last_failure: Mutex::new(None),
        }
    }

    fn check(&self, interactive: bool, now: Instant) -> Result<(), &'static str> {
        if let Ok(last_failure) = self.last_failure.lock() {
            if last_failure.is_some_and(|at| now.duration_since(at) < FAILURE_BACKOFF) {
                return Err(BACKOFF_ERROR_CODE);
            }
        }
        if !interactive {
            if let Ok(last_attempt) = self.last_attempt.lock() {
                if last_attempt.is_some_and(|at| now.duration_since(at) < BACKGROUND_MIN_INTERVAL) {
                    return Err(THROTTLED_ERROR_CODE);
                }
            }
        }
        Ok(())
    }

    fn mark_attempt(&self, now: Instant) {
        if let Ok(mut last_attempt) = self.last_attempt.lock() {
            *last_attempt = Some(now);
        }
    }

    fn mark_failure(&self, now: Instant) {
        if let Ok(mut last_failure) = self.last_failure.lock() {
            *last_failure = Some(now);
        }
    }
}

fn probe_gate() -> &'static ProbeGate {
    static GATE: OnceLock<ProbeGate> = OnceLock::new();
    GATE.get_or_init(ProbeGate::new)
}

// ── 层级入口 ──────────────────────────────────────────────

/// Claude 订阅额度的 CLI 探测层。失败以 `Err(稳定错误码)` 表达，由回退链降级。
pub(crate) async fn probe_claude_usage(interactive: bool) -> Result<SubscriptionQuota, String> {
    let now = Instant::now();
    if let Err(code) = probe_gate().check(interactive, now) {
        return Err(code.to_string());
    }
    probe_gate().mark_attempt(now);

    let home = crate::config::get_home_dir();
    let binary = resolve_claude_binary(&home);
    let Some(binary) = binary else {
        probe_gate().mark_failure(Instant::now());
        return Err(NOT_INSTALLED_ERROR_CODE.to_string());
    };

    let claude_config_dir = crate::settings::get_claude_override_dir();
    let result =
        tokio::task::spawn_blocking(move || run_probe(&binary, &home, claude_config_dir.as_deref()))
            .await;

    match result {
        Ok(Ok(usage)) => Ok(usage.into_quota()),
        _ => {
            probe_gate().mark_failure(Instant::now());
            Err(FAILED_ERROR_CODE.to_string())
        }
    }
}

fn resolve_claude_binary(home: &std::path::Path) -> Option<PathBuf> {
    crate::services::claude_cli_auth::claude_binary_candidates_for_home(home)
        .into_iter()
        .find(|candidate| {
            // 裸名字("claude")走 PATH 解析,不检查存在性;带目录的候选必须真实存在。
            candidate
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .is_none_or(|parent| parent.exists())
        })
}

// ── PTY 探测 ──────────────────────────────────────────────

fn run_probe(
    binary: &std::path::Path,
    home: &std::path::Path,
    claude_config_dir: Option<&std::path::Path>,
) -> Result<ProbeUsage, String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|_| FAILED_ERROR_CODE.to_string())?;

    let mut command = CommandBuilder::new(binary);
    command.args([
        "--allowed-tools",
        "",
        "--strict-mcp-config",
        "--session-id",
        &uuid::Uuid::new_v4().to_string(),
    ]);
    command.env("DISABLE_AUTOUPDATER", "1");
    if let Some(config_dir) = claude_config_dir {
        command.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    command.cwd(home);

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|_| FAILED_ERROR_CODE.to_string())?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|_| FAILED_ERROR_CODE.to_string())?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|_| FAILED_ERROR_CODE.to_string())?;

    // 等 CLI 完成启动渲染,再发送 /usage。
    std::thread::sleep(Duration::from_millis(1_500));
    let _ = writer.write_all(b"/usage\r");

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut last_ping = Instant::now();
    let mut buffer = Vec::new();
    let mut read_buf = [0u8; 8192];
    loop {
        match reader.read(&mut read_buf) {
            Ok(0) => break,
            Ok(n) => buffer.extend_from_slice(&read_buf[..n]),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
        let text = String::from_utf8_lossy(&buffer);
        let parsed = parse_claude_usage_text(&text);
        if parsed.is_some() || text.contains("Failed to load usage data") {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        if last_ping.elapsed() >= Duration::from_millis(800) {
            let _ = writer.write_all(b"\r");
            last_ping = Instant::now();
        }
        if child
            .try_wait()
            .map(|status| status.is_some())
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();

    let text = String::from_utf8_lossy(&buffer);
    match parse_claude_usage_text(&text) {
        Some(usage) => Ok(usage),
        None => Err(FAILED_ERROR_CODE.to_string()),
    }
}

// ── 文本解析（纯函数） ────────────────────────────────────

pub(crate) struct ProbeUsage {
    pub tiers: Vec<QuotaTier>,
}

impl ProbeUsage {
    fn into_quota(self) -> SubscriptionQuota {
        SubscriptionQuota {
            tool: TOOL_LABEL.to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: self.tiers,
            plan_type: None,
            plan_renews_at: None,
            manual_reset_credits: None,
            extra_usage: None,
            error: None,
            queried_at: Some(now_millis()),
        }
    }
}

fn strip_ansi(raw: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("static ANSI regex");
    re.replace_all(raw, "").into_owned()
}

/// 解析 `/usage` 面板文本。任一主窗口（5 小时/7 天）出现即成功；
/// Opus/Sonnet 窗口是附带解析、缺失不影响结果。
pub(crate) fn parse_claude_usage_text(raw: &str) -> Option<ProbeUsage> {
    let text = strip_ansi(raw);
    if text.contains("Failed to load usage data") {
        return None;
    }

    let mut tiers = Vec::new();
    extract_window(&text, "Current session", TIER_FIVE_HOUR, &mut tiers);
    extract_window(
        &text,
        "Current week (all models)",
        TIER_SEVEN_DAY,
        &mut tiers,
    );
    extract_window(
        &text,
        "Current week (Opus)",
        TIER_SEVEN_DAY_OPUS,
        &mut tiers,
    );
    // 覆盖 "Current week (Sonnet only)" 与 "Current week (Sonnet)" 两种写法。
    extract_window(
        &text,
        "Current week (Sonnet",
        TIER_SEVEN_DAY_SONNET,
        &mut tiers,
    );

    let has_primary = tiers
        .iter()
        .any(|tier| tier.name == TIER_FIVE_HOUR || tier.name == TIER_SEVEN_DAY);
    has_primary.then_some(ProbeUsage { tiers })
}

fn extract_window(text: &str, label: &str, tier_name: &str, tiers: &mut Vec<QuotaTier>) {
    let Some(start) = text.find(label) else { return };
    // 百分比渲染在标签之后的短窗口内;限制段长避免吃到下一个面板的数字。
    let segment = &text[start..(start + 400).min(text.len())];
    let percent_re = regex::Regex::new(r"(\d+(?:\.\d+)?)%").expect("static percent regex");
    let Some(captures) = percent_re.captures(segment) else {
        return;
    };
    let Some(percent) = captures.get(1).and_then(|m| m.as_str().parse::<f64>().ok()) else {
        return;
    };
    tiers.push(QuotaTier {
        name: tier_name.to_string(),
        utilization: percent.clamp(0.0, 100.0),
        resets_at: None,
        used_value_usd: None,
        max_value_usd: None,
    });
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

    #[test]
    fn parses_current_session_and_weekly_windows() {
        let fixture = "\u{1b}[32m  Current session\u{1b}[0m\n\
                       \u{1b}[1m     42%\u{1b}[0m used\n\
                       Current week (all models)\n\
                       \u{1b}[1m     78%\u{1b}[0m used\n";
        let usage = parse_claude_usage_text(fixture).expect("windows parse");
        assert_eq!(usage.tiers.len(), 2);
        assert_eq!(usage.tiers[0].name, TIER_FIVE_HOUR);
        assert_eq!(usage.tiers[0].utilization, 42.0);
        assert_eq!(usage.tiers[1].name, TIER_SEVEN_DAY);
        assert_eq!(usage.tiers[1].utilization, 78.0);
    }

    #[test]
    fn parses_extra_model_windows_when_present() {
        let fixture = "Current session\n  10% used\n\
                       Current week (all models)\n  20% used\n\
                       Current week (Opus)\n  30% used\n\
                       Current week (Sonnet only)\n  40% used\n";
        let usage = parse_claude_usage_text(fixture).expect("windows parse");
        let names: Vec<&str> = usage.tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [TIER_FIVE_HOUR, TIER_SEVEN_DAY, TIER_SEVEN_DAY_OPUS, TIER_SEVEN_DAY_SONNET]
        );
        assert_eq!(usage.tiers[3].utilization, 40.0);
    }

    #[test]
    fn rejects_failed_panel_and_garbage() {
        let failed = "Current session\nLoading...\nFailed to load usage data\n";
        assert!(parse_claude_usage_text(failed).is_none());
        assert!(parse_claude_usage_text("").is_none());
        assert!(parse_claude_usage_text("unrelated terminal noise").is_none());
    }

    #[test]
    fn requires_a_primary_window() {
        // 只有 Opus 行而没有 5 小时/7 天窗口:没有展示价值,判失败。
        let fixture = "Current week (Opus)\n  30% used\n";
        assert!(parse_claude_usage_text(fixture).is_none());
    }
}
