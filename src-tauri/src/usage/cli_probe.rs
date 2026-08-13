//! CLI 用量探测:回退链的第二层。
//!
//! 在真终端(PTY)里拉起 CLI,向 TUI 发送命令,解析渲染出来的用量百分比。
//! - Claude Code:发送 `/usage`,解析 "Current session" / "Current week (all models)"
//!   及 Opus/Sonnet 窗口(与 CodexBar 的 ClaudeStatusProbe 机制一致);
//! - Codex:发送 `/status`,解析 Credits / 5h limit / Weekly limit / Monthly credit
//!   limit(与 CodexBar 的 CodexStatusProbe 机制一致)。
//!
//! 探测很重(拉起一个交互式 CLI 进程),因此:
//! - 调度器(后台)路径上两次探测之间至少间隔 30 分钟(两个 CLI 各自独立计数);
//! - 用户手动刷新有 60 秒最小间隔(防狂点反复拉起 CLI),但仍豁免后台长间隔;
//! - 任何失败只产生稳定错误码,绝不含终端原文(可能带敏感信息)。
//!
//! 文本解析是纯函数(`parse_claude_usage_text` / `parse_codex_status_text`),
//! 测试用夹具文本覆盖,不真实拉起进程。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::subscription::{
    QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY, TIER_SEVEN_DAY_OPUS,
    TIER_SEVEN_DAY_SONNET,
};

/// 探测总超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// 调度器(非交互)路径上的最小探测间隔。
const BACKGROUND_MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// 手动刷新路径上的最小探测间隔:防用户狂点刷新反复拉起 CLI 进程。
const INTERACTIVE_MIN_INTERVAL: Duration = Duration::from_secs(60);
/// 探测失败后的退避(对交互与后台统一生效,避免反复拉起失败的 CLI)。
const FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// 催重绘的回车最多补发几次。见 `run_tty_probe` 里的说明。
const MAX_REDRAW_PINGS: u8 = 3;

/// 错误码(稳定、前端可映射;不带终端原文)。
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
        // 交互豁免后台的 30 分钟长间隔,但仍有 60 秒最小间隔兜底。
        let min_interval = if interactive {
            INTERACTIVE_MIN_INTERVAL
        } else {
            BACKGROUND_MIN_INTERVAL
        };
        if let Ok(last_attempt) = self.last_attempt.lock() {
            if last_attempt.is_some_and(|at| now.duration_since(at) < min_interval) {
                return Err(THROTTLED_ERROR_CODE);
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

fn claude_probe_gate() -> &'static ProbeGate {
    static GATE: OnceLock<ProbeGate> = OnceLock::new();
    GATE.get_or_init(ProbeGate::new)
}

fn codex_probe_gate() -> &'static ProbeGate {
    static GATE: OnceLock<ProbeGate> = OnceLock::new();
    GATE.get_or_init(ProbeGate::new)
}

// ── 层级入口 ──────────────────────────────────────────────

/// Claude 订阅额度的 CLI 探测层。失败以 `Err(稳定错误码)` 表达,由回退链降级。
pub(crate) async fn probe_claude_usage(interactive: bool) -> Result<SubscriptionQuota, String> {
    let now = Instant::now();
    if let Err(code) = claude_probe_gate().check(interactive, now) {
        return Err(code.to_string());
    }
    claude_probe_gate().mark_attempt(now);

    let home = crate::config::get_home_dir();
    let binary = resolve_claude_binary(&home);
    let Some(binary) = binary else {
        claude_probe_gate().mark_failure(Instant::now());
        return Err(NOT_INSTALLED_ERROR_CODE.to_string());
    };

    let claude_config_dir = crate::settings::get_claude_override_dir();
    let result = tokio::task::spawn_blocking(move || -> Result<ProbeUsage, String> {
        let mut env: Vec<(&str, String)> = Vec::new();
        if let Some(dir) = claude_config_dir.as_deref() {
            env.push(("CLAUDE_CONFIG_DIR", dir.to_string_lossy().into_owned()));
        }
        let env = if env.is_empty() {
            None
        } else {
            Some(env.as_slice())
        };
        let text = run_tty_probe(
            &binary,
            &[
                "--allowed-tools",
                "",
                "--strict-mcp-config",
                "--session-id",
                &uuid::Uuid::new_v4().to_string(),
            ],
            &home,
            env,
            b"/usage\r",
            PROBE_TIMEOUT,
            |text| {
                parse_claude_usage_text(text).is_some()
                    || text.contains("Failed to load usage data")
            },
        )?;
        Ok(ProbeUsage {
            tiers: parse_claude_usage_text(&text).ok_or_else(|| FAILED_ERROR_CODE.to_string())?,
        })
    })
    .await;

    match result {
        Ok(Ok(usage)) => Ok(usage.into_quota("claude_probe")),
        _ => {
            claude_probe_gate().mark_failure(Instant::now());
            Err(FAILED_ERROR_CODE.to_string())
        }
    }
}

/// Codex 订阅额度的 CLI 探测层。失败以 `Err(稳定错误码)` 表达,由回退链降级。
pub(crate) async fn probe_codex_status(interactive: bool) -> Result<SubscriptionQuota, String> {
    let now = Instant::now();
    if let Err(code) = codex_probe_gate().check(interactive, now) {
        return Err(code.to_string());
    }
    codex_probe_gate().mark_attempt(now);

    let home = crate::config::get_home_dir();
    let binary = resolve_codex_binary(&home);
    let Some(binary) = binary else {
        codex_probe_gate().mark_failure(Instant::now());
        return Err(NOT_INSTALLED_ERROR_CODE.to_string());
    };

    let result = tokio::task::spawn_blocking(move || -> Result<ProbeUsage, String> {
        let text = run_tty_probe(
            &binary,
            &[
                "-s",
                "read-only",
                "-a",
                "untrusted",
                "-c",
                "history.persistence=\"none\"",
            ],
            &home,
            None,
            b"/status\r",
            PROBE_TIMEOUT,
            |text| {
                parse_codex_status_text(text).is_some() || text.contains("data not available yet")
            },
        )?;
        Ok(ProbeUsage {
            tiers: parse_codex_status_text(&text).ok_or_else(|| FAILED_ERROR_CODE.to_string())?,
        })
    })
    .await;

    match result {
        Ok(Ok(usage)) => Ok(usage.into_quota("codex_probe")),
        _ => {
            codex_probe_gate().mark_failure(Instant::now());
            Err(FAILED_ERROR_CODE.to_string())
        }
    }
}

/// 在 `PATH` 里找可执行文件。从 Finder 启动的 GUI app 只继承
/// `/usr/bin:/bin:/usr/sbin:/sbin`,所以这一步经常落空——落空后必须继续走
/// 绝对路径候选,不能就此判定「没装」。
fn find_in_path(bare: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bare))
        .find(|candidate| candidate.is_file())
}

/// 候选里挑一个真能跑的:先 PATH(用户在终端里用的就是它),再逐个检查绝对
/// 路径候选**自身**是否存在。注意不能按「父目录存在」筛选——裸名字的父目录
/// 是空串,那样第一个候选永远命中,后面的绝对路径全成死代码。
fn resolve_probe_binary(bare: &str, candidates: Vec<PathBuf>) -> Option<PathBuf> {
    find_in_path(bare).or_else(|| {
        candidates
            .into_iter()
            .filter(|candidate| {
                candidate
                    .parent()
                    .is_some_and(|parent| !parent.as_os_str().is_empty())
            })
            .find(|candidate| candidate.is_file())
    })
}

fn resolve_claude_binary(home: &std::path::Path) -> Option<PathBuf> {
    resolve_probe_binary(
        "claude",
        crate::services::claude_cli_auth::claude_binary_candidates_for_home(home),
    )
}

fn resolve_codex_binary(home: &std::path::Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for relative in [
        ".local/bin/codex",
        ".local/share/pnpm/codex",
        ".npm-global/bin/codex",
        ".volta/bin/codex",
    ] {
        candidates.push(home.join(relative));
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
        candidates.push(PathBuf::from("/usr/local/bin/codex"));
    }
    resolve_probe_binary("codex", candidates)
}

// ── PTY 探测(Claude / Codex 共用) ─────────────────────────

/// 在 PTY 里拉起 `binary`,等启动渲染后发送 `initial_input`,轮询输出直到
/// `is_done` 成立、进程退出或超时;返回捕获到的原始终端文本。
fn run_tty_probe(
    binary: &std::path::Path,
    args: &[&str],
    home: &std::path::Path,
    extra_env: Option<&[(&str, String)]>,
    initial_input: &[u8],
    timeout: Duration,
    is_done: impl Fn(&str) -> bool,
) -> Result<String, String> {
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
    command.args(args);
    command.env("DISABLE_AUTOUPDATER", "1");
    if let Some(env) = extra_env {
        for (key, value) in env {
            command.env(key, value);
        }
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

    // PTY 的 read 是阻塞的:子进程一旦不再输出,read 就永远不返回,循环里的
    // deadline 检查也就永远轮不到——超时形同虚设,还会长期占住一个 blocking
    // 线程。所以把读放到独立线程里,主循环只按超时收管道。
    let (sender, receiver) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut read_buf = [0u8; 8192];
        loop {
            match reader.read(&mut read_buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sender.send(read_buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // 等 CLI 完成启动渲染,再发送命令。
    std::thread::sleep(Duration::from_millis(1_500));
    let _ = writer.write_all(initial_input);

    let deadline = Instant::now() + timeout;
    let mut last_ping = Instant::now();
    let mut pings_sent = 0u8;
    let mut buffer = Vec::new();
    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => buffer.extend_from_slice(&chunk),
            // 读线程结束 = PTY 关闭,不会再有输出了。
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let text = String::from_utf8_lossy(&buffer);
        if is_done(&text) {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        // 回车是催 TUI 重绘用的,但它同时也会「确认」CLI 可能正停在的对话框
        // (信任目录、onboarding 等),所以只补发有限几次,不无限敲。
        if pings_sent < MAX_REDRAW_PINGS && last_ping.elapsed() >= Duration::from_millis(800) {
            let _ = writer.write_all(b"\r");
            last_ping = Instant::now();
            pings_sent += 1;
        }
        if child
            .try_wait()
            .map(|status| status.is_some())
            .unwrap_or(false)
        {
            break;
        }
    }
    let _ = child.kill();
    // master 在函数结束时析构,读线程随之收到 EOF 退出,不会泄漏。

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

// ── 文本解析(纯函数) ────────────────────────────────────

pub(crate) struct ProbeUsage {
    pub tiers: Vec<QuotaTier>,
}

impl ProbeUsage {
    fn into_quota(self, tool: &str) -> SubscriptionQuota {
        SubscriptionQuota {
            success: true,
            tiers: self.tiers,
            queried_at: Some(now_millis()),
            ..SubscriptionQuota::skeleton(tool)
        }
    }
}

fn strip_ansi(raw: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("static ANSI regex");
    re.replace_all(raw, "").into_owned()
}

/// 解析 `/usage` 面板文本。任一主窗口(5 小时/7 天)出现即成功;
/// Opus/Sonnet 窗口是附带解析、缺失不影响结果。
pub(crate) fn parse_claude_usage_text(raw: &str) -> Option<Vec<QuotaTier>> {
    let text = strip_ansi(raw);
    if text.contains("Failed to load usage data") {
        return None;
    }

    let mut tiers = Vec::new();
    extract_percent_after(&text, "Current session", TIER_FIVE_HOUR, &mut tiers);
    extract_percent_after(
        &text,
        "Current week (all models)",
        TIER_SEVEN_DAY,
        &mut tiers,
    );
    extract_percent_after(
        &text,
        "Current week (Opus)",
        TIER_SEVEN_DAY_OPUS,
        &mut tiers,
    );
    // 覆盖 "Current week (Sonnet only)" 与 "Current week (Sonnet)" 两种写法。
    extract_percent_after(
        &text,
        "Current week (Sonnet",
        TIER_SEVEN_DAY_SONNET,
        &mut tiers,
    );

    let has_primary = tiers
        .iter()
        .any(|tier| tier.name == TIER_FIVE_HOUR || tier.name == TIER_SEVEN_DAY);
    has_primary.then_some(tiers)
}

/// 解析 `codex /status` 文本。任一主窗口(5h/Weekly)出现即成功;
/// Credits 与 Monthly credit limit 是附带解析。
pub(crate) fn parse_codex_status_text(raw: &str) -> Option<Vec<QuotaTier>> {
    let text = strip_ansi(raw);
    if text.contains("data not available yet") {
        return None;
    }

    let mut tiers = Vec::new();
    extract_percent_after(&text, "5h limit", TIER_FIVE_HOUR, &mut tiers);
    extract_percent_after(&text, "Weekly limit", TIER_SEVEN_DAY, &mut tiers);
    extract_monthly_credit_limit(&text, &mut tiers);

    let has_primary = tiers
        .iter()
        .any(|tier| tier.name == TIER_FIVE_HOUR || tier.name == TIER_SEVEN_DAY);
    has_primary.then_some(tiers)
}

/// 在 `label` 之后的短窗口内找第一个百分比。限制段长避免吃到下一个面板的数字。
fn extract_percent_after(text: &str, label: &str, tier_name: &str, tiers: &mut Vec<QuotaTier>) {
    let Some(start) = text.find(label) else {
        return;
    };
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

/// `Monthly credit limit: X of Y credits used` → 已用百分比 tier。
fn extract_monthly_credit_limit(text: &str, tiers: &mut Vec<QuotaTier>) {
    let Some(start) = text.find("Monthly credit limit") else {
        return;
    };
    let segment = &text[start..(start + 300).min(text.len())];
    let used_re = regex::Regex::new(r"(\d[\d., ]*)\s+of\s+(\d[\d., ]*)").expect("static regex");
    let Some(captures) = used_re.captures(segment) else {
        return;
    };
    let parse = |value: &str| {
        value
            .replace([' ', ','], "")
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    let used = captures.get(1).map(|m| m.as_str()).and_then(parse);
    let limit = captures.get(2).map(|m| m.as_str()).and_then(parse);
    let (Some(used), Some(limit)) = (used, limit) else {
        return;
    };
    if limit <= 0.0 {
        return;
    }
    tiers.push(QuotaTier {
        name: "codex_credit_limit".to_string(),
        utilization: (used / limit * 100.0).clamp(0.0, 100.0),
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

    // ── 可执行文件解析 ──────────────────────────────────

    /// 回归:曾经按「父目录是否存在」筛选候选。裸名字的父目录是空串,
    /// `is_none_or` 直接放行,于是永远命中第一个候选,后面所有绝对路径
    /// 都是死代码——从 Finder 启动的 app PATH 里没有 claude/codex 时就废了。
    #[test]
    fn resolve_skips_candidates_whose_file_does_not_exist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("nonexistent/tool");
        let present_dir = temp.path().join("real");
        std::fs::create_dir_all(&present_dir).expect("mkdir");
        let present = present_dir.join("tool");
        std::fs::write(&present, b"#!/bin/sh\n").expect("write");

        // 用一个 PATH 里不可能存在的名字,强制走绝对路径候选。
        let resolved = resolve_probe_binary(
            "llm-usage-bar-no-such-binary",
            vec![missing.clone(), present.clone()],
        );
        assert_eq!(resolved.as_deref(), Some(present.as_path()));
    }

    #[test]
    fn resolve_returns_none_when_nothing_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(resolve_probe_binary(
            "llm-usage-bar-no-such-binary",
            vec![temp.path().join("missing/tool")],
        )
        .is_none());
    }

    // ── 探测闸门 ────────────────────────────────────────

    #[test]
    fn gate_background_interval_is_long_and_interactive_is_short() {
        let gate = ProbeGate::new();
        let t0 = Instant::now();
        // 首次后台尝试放行。
        assert!(gate.check(false, t0).is_ok());
        gate.mark_attempt(t0);
        // 后台 30 分钟内节流;交互只豁免到 60 秒。
        assert_eq!(
            gate.check(false, t0 + Duration::from_secs(60)),
            Err(THROTTLED_ERROR_CODE)
        );
        assert_eq!(
            gate.check(true, t0 + Duration::from_secs(30)),
            Err(THROTTLED_ERROR_CODE)
        );
        assert!(gate.check(true, t0 + Duration::from_secs(61)).is_ok());
        assert_eq!(
            gate.check(false, t0 + Duration::from_secs(29 * 60)),
            Err(THROTTLED_ERROR_CODE)
        );
        assert!(gate.check(false, t0 + Duration::from_secs(30 * 60)).is_ok());
    }

    #[test]
    fn gate_failure_backoff_applies_to_interactive_too() {
        let gate = ProbeGate::new();
        let t0 = Instant::now();
        gate.mark_failure(t0);
        assert_eq!(
            gate.check(true, t0 + Duration::from_secs(30)),
            Err(BACKOFF_ERROR_CODE)
        );
        assert!(gate.check(true, t0 + Duration::from_secs(5 * 60)).is_ok());
    }

    // ── Claude /usage ────────────────────────────────────

    #[test]
    fn parses_current_session_and_weekly_windows() {
        let fixture = "\u{1b}[32m  Current session\u{1b}[0m\n\
                       \u{1b}[1m     42%\u{1b}[0m used\n\
                       Current week (all models)\n\
                       \u{1b}[1m     78%\u{1b}[0m used\n";
        let tiers = parse_claude_usage_text(fixture).expect("windows parse");
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, TIER_FIVE_HOUR);
        assert_eq!(tiers[0].utilization, 42.0);
        assert_eq!(tiers[1].name, TIER_SEVEN_DAY);
        assert_eq!(tiers[1].utilization, 78.0);
    }

    #[test]
    fn parses_extra_model_windows_when_present() {
        let fixture = "Current session\n  10% used\n\
                       Current week (all models)\n  20% used\n\
                       Current week (Opus)\n  30% used\n\
                       Current week (Sonnet only)\n  40% used\n";
        let tiers = parse_claude_usage_text(fixture).expect("windows parse");
        let names: Vec<&str> = tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                TIER_FIVE_HOUR,
                TIER_SEVEN_DAY,
                TIER_SEVEN_DAY_OPUS,
                TIER_SEVEN_DAY_SONNET
            ]
        );
        assert_eq!(tiers[3].utilization, 40.0);
    }

    #[test]
    fn claude_rejects_failed_panel_and_garbage() {
        let failed = "Current session\nLoading...\nFailed to load usage data\n";
        assert!(parse_claude_usage_text(failed).is_none());
        assert!(parse_claude_usage_text("").is_none());
        assert!(parse_claude_usage_text("unrelated terminal noise").is_none());
    }

    #[test]
    fn claude_requires_a_primary_window() {
        // 只有 Opus 行而没有 5 小时/7 天窗口:没有展示价值,判失败。
        let fixture = "Current week (Opus)\n  30% used\n";
        assert!(parse_claude_usage_text(fixture).is_none());
    }

    // ── Codex /status ────────────────────────────────────

    #[test]
    fn parses_codex_status_windows() {
        let fixture = "\u{1b}[0m Credits: $12.34\n\
                       5h limit: 45% used · Resets in 2h 30m\n\
                       Weekly limit: 60% used · Resets in 3 days\n\
                       Monthly credit limit: 120 of 400 credits used\n";
        let tiers = parse_codex_status_text(fixture).expect("status parses");
        let five = tiers.iter().find(|t| t.name == TIER_FIVE_HOUR).unwrap();
        assert_eq!(five.utilization, 45.0);
        let weekly = tiers.iter().find(|t| t.name == TIER_SEVEN_DAY).unwrap();
        assert_eq!(weekly.utilization, 60.0);
        let monthly = tiers
            .iter()
            .find(|t| t.name == "codex_credit_limit")
            .unwrap();
        assert_eq!(monthly.utilization, 30.0);
    }

    #[test]
    fn codex_rejects_unavailable_and_garbage() {
        assert!(parse_codex_status_text("data not available yet").is_none());
        assert!(parse_codex_status_text("").is_none());
        assert!(parse_codex_status_text("prompt> ").is_none());
    }

    #[test]
    fn codex_requires_a_primary_window() {
        // 只有 Credits 而没有窗口百分比:判失败。
        let fixture = "Credits: $12.34\n";
        assert!(parse_codex_status_text(fixture).is_none());
    }
}
