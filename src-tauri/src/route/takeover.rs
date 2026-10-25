//! BE-17: Claude takeover and `detect_tools` (C3 / C22).
//!
//! Detection never returns an error when auth cannot be read. Subscription
//! `apply_takeover` does not write. Codex subscription traffic must not go
//! through `pointer.rs`; that writer is the API-key path only.
//!
//! Windows Credential Manager is intentionally not read. The command layer
//! returns a stub on Windows.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_paths::{
    extract_codex_auth_api_key, extract_codex_experimental_bearer_token,
    is_custom_codex_model_provider_id,
};
use crate::error::AppError;
use crate::route::pointer::{
    disconnect_codex_inner, point_codex_at_router_inner, ROUTER_PROVIDER_ID,
};

const BACKUP_SUFFIX: &str = ".lub-takeover-backup";
const ABSENT_SUFFIX: &str = ".lub-takeover-absent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    Subscription,
    Unknown,
    #[serde(rename = "n_a")]
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAuthView {
    pub mode: AuthMode,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDetectRow {
    pub tool: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub auth_mode: AuthMode,
    pub proxied: bool,
    pub enforceable: bool,
    pub note_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyTakeoverResult {
    pub note_codes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DetectEnv {
    pub anthropic_api_key: Option<String>,
    pub anthropic_auth_token: Option<String>,
    pub openai_api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TakeoverLayout {
    pub home: PathBuf,
    pub bin_dirs: Vec<PathBuf>,
    pub router_port: u16,
    pub env: DetectEnv,
}

pub fn macos_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin"),
        home.join(".volta/bin"),
        home.join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

impl TakeoverLayout {
    fn claude_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }

    fn claude_settings(&self) -> PathBuf {
        self.claude_dir().join("settings.json")
    }

    fn claude_account(&self) -> PathBuf {
        self.home.join(".claude.json")
    }

    fn codex_dir(&self) -> PathBuf {
        self.home.join(".codex")
    }

    fn codex_config(&self) -> PathBuf {
        self.codex_dir().join("config.toml")
    }

    fn codex_auth(&self) -> PathBuf {
        self.codex_dir().join("auth.json")
    }
}

pub fn windows_tool_stub() -> Vec<ToolDetectRow> {
    // TODO(BE-17): Windows install and Credential Manager detection stay unimplemented.
    ["claude", "codex", "cursor"]
        .into_iter()
        .map(|tool| {
            let cursor = tool == "cursor";
            ToolDetectRow {
                tool: tool.to_string(),
                installed: false,
                version: None,
                auth_mode: if cursor {
                    AuthMode::NotApplicable
                } else {
                    AuthMode::Unknown
                },
                proxied: false,
                enforceable: !cursor,
                note_codes: vec!["windows_detection_pending".to_string()],
            }
        })
        .collect()
}

pub fn detect_claude_auth_at(layout: &TakeoverLayout) -> ClaudeAuthView {
    let mut evidence = Vec::new();
    let mut api_key = false;
    if nonempty(&layout.env.anthropic_api_key) {
        api_key = true;
        evidence.push("process_env_api_key".to_string());
    }
    if nonempty(&layout.env.anthropic_auth_token) {
        api_key = true;
        evidence.push("process_env_auth_token".to_string());
    }
    match read_json_optional(&layout.claude_settings()) {
        JsonRead::Value(value) => {
            if nonempty_at(&value, &["env", "ANTHROPIC_API_KEY"]) {
                api_key = true;
                evidence.push("settings_env_api_key".to_string());
            }
            if nonempty_at(&value, &["env", "ANTHROPIC_AUTH_TOKEN"]) {
                api_key = true;
                evidence.push("settings_env_auth_token".to_string());
            }
        }
        JsonRead::Missing => {}
        JsonRead::Unreadable => evidence.push("settings_unreadable".to_string()),
    }
    let mut subscription = false;
    match read_json_optional(&layout.claude_account()) {
        JsonRead::Value(value) => {
            if nonempty_at(&value, &["oauthAccount", "organizationType"]) {
                subscription = true;
                evidence.push("account_plan".to_string());
            }
        }
        JsonRead::Missing => {}
        JsonRead::Unreadable => evidence.push("account_unreadable".to_string()),
    }
    let mode = if api_key {
        AuthMode::ApiKey
    } else if subscription {
        AuthMode::Subscription
    } else {
        AuthMode::Unknown
    };
    ClaudeAuthView { mode, evidence }
}

pub fn detect_tools_at(layout: &TakeoverLayout) -> Vec<ToolDetectRow> {
    vec![claude_row(layout), codex_row(layout), cursor_row(layout)]
}

pub fn apply_takeover_at(
    layout: &TakeoverLayout,
    tool: &str,
    enable: bool,
) -> Result<ApplyTakeoverResult, AppError> {
    match tool {
        "cursor" => Err(unsupported_tool()),
        "claude" => apply_claude(layout, enable),
        "codex" => apply_codex(layout, enable),
        _ => Err(unsupported_tool()),
    }
}

pub fn set_claude_subscription_proxy_at(
    layout: &TakeoverLayout,
    enabled: bool,
    terms_ack_at: Option<i64>,
) -> Result<(), AppError> {
    if enabled {
        let Some(ack) = terms_ack_at else {
            return Err(terms_required());
        };
        write_claude_base_url(layout, true)?;
        log::info!("claude subscription proxy acknowledged; termsAckAt={ack}");
        Ok(())
    } else {
        restore_claude_settings(&layout.claude_settings())?;
        Ok(())
    }
}

fn apply_claude(layout: &TakeoverLayout, enable: bool) -> Result<ApplyTakeoverResult, AppError> {
    match detect_claude_auth_at(layout).mode {
        AuthMode::Subscription => Err(terms_required()),
        AuthMode::Unknown if enable => Ok(ApplyTakeoverResult {
            note_codes: vec!["auth_unknown_skipped".to_string()],
        }),
        AuthMode::ApiKey | AuthMode::Unknown | AuthMode::NotApplicable => {
            if enable {
                write_claude_base_url(layout, false)?;
            } else {
                restore_claude_settings(&layout.claude_settings())?;
            }
            Ok(ApplyTakeoverResult {
                note_codes: Vec::new(),
            })
        }
    }
}

fn apply_codex(layout: &TakeoverLayout, enable: bool) -> Result<ApplyTakeoverResult, AppError> {
    match codex_auth(layout).0 {
        AuthMode::Subscription => Err(terms_required()),
        AuthMode::Unknown if enable => Ok(ApplyTakeoverResult {
            note_codes: vec!["auth_unknown_skipped".to_string()],
        }),
        AuthMode::ApiKey | AuthMode::Unknown | AuthMode::NotApplicable => {
            let config = layout.codex_config();
            if enable {
                if !codex_points_at_router(&config, layout.router_port) {
                    point_codex_at_router_inner(&config, layout.router_port)
                        .map_err(|_| config_write_failed())?;
                }
            } else if codex_points_at_router(&config, layout.router_port) {
                disconnect_codex_inner(&config).map_err(|_| config_write_failed())?;
            }
            Ok(ApplyTakeoverResult {
                note_codes: Vec::new(),
            })
        }
    }
}

fn claude_row(layout: &TakeoverLayout) -> ToolDetectRow {
    let auth = detect_claude_auth_at(layout);
    let proxied = claude_base_url(layout).is_some_and(|url| is_loopback_url(&url));
    tool_row("claude", claude_installed(layout), auth.mode, proxied)
}

fn codex_row(layout: &TakeoverLayout) -> ToolDetectRow {
    let (mode, _) = codex_auth(layout);
    tool_row(
        "codex",
        dir_or_binary(&layout.codex_dir(), "codex", &layout.bin_dirs),
        mode,
        codex_proxied(layout, mode),
    )
}

fn cursor_row(layout: &TakeoverLayout) -> ToolDetectRow {
    let installed = layout.home.join(".cursor").is_dir()
        || layout
            .home
            .join("Library/Application Support/Cursor")
            .is_dir()
        || binary_exists("cursor", &layout.bin_dirs);
    ToolDetectRow {
        tool: "cursor".to_string(),
        installed,
        version: None,
        auth_mode: AuthMode::NotApplicable,
        proxied: false,
        enforceable: false,
        note_codes: installed_notes(installed, AuthMode::NotApplicable, false),
    }
}

fn tool_row(tool: &str, installed: bool, mode: AuthMode, proxied: bool) -> ToolDetectRow {
    let enforceable = matches!(mode, AuthMode::ApiKey | AuthMode::Unknown);
    ToolDetectRow {
        tool: tool.to_string(),
        installed,
        version: None,
        auth_mode: mode,
        proxied,
        enforceable,
        note_codes: installed_notes(installed, mode, proxied),
    }
}

fn installed_notes(installed: bool, mode: AuthMode, proxied: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if !installed {
        notes.push("not_installed".to_string());
    }
    match mode {
        AuthMode::Unknown => notes.push("auth_unknown".to_string()),
        AuthMode::Subscription if !proxied => notes.push("subscription_not_proxied".to_string()),
        AuthMode::ApiKey if !proxied => notes.push("api_key_not_proxied".to_string()),
        _ => {}
    }
    notes
}

fn claude_installed(layout: &TakeoverLayout) -> bool {
    dir_or_binary(&layout.claude_dir(), "claude", &layout.bin_dirs)
}

fn dir_or_binary(dir: &Path, binary: &str, bin_dirs: &[PathBuf]) -> bool {
    dir.is_dir() || binary_exists(binary, bin_dirs)
}

fn binary_exists(name: &str, bin_dirs: &[PathBuf]) -> bool {
    bin_dirs.iter().any(|dir| dir.join(name).is_file())
}

fn codex_auth(layout: &TakeoverLayout) -> (AuthMode, Vec<String>) {
    let mut evidence = Vec::new();
    if nonempty(&layout.env.openai_api_key) {
        evidence.push("process_env_api_key".to_string());
        return (AuthMode::ApiKey, evidence);
    }
    let auth = match read_json_optional(&layout.codex_auth()) {
        JsonRead::Value(value) => Some(value),
        JsonRead::Missing => None,
        JsonRead::Unreadable => {
            evidence.push("auth_unreadable".to_string());
            return (AuthMode::Unknown, evidence);
        }
    };
    if auth.as_ref().and_then(extract_codex_auth_api_key).is_some() {
        evidence.push("auth_json_api_key".to_string());
        return (AuthMode::ApiKey, evidence);
    }
    let config_text = match fs::read_to_string(layout.codex_config()) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            evidence.push("config_unreadable".to_string());
            return (AuthMode::Unknown, evidence);
        }
    };
    if config_text
        .as_deref()
        .and_then(extract_codex_experimental_bearer_token)
        .is_some()
    {
        evidence.push("config_bearer".to_string());
        return (AuthMode::ApiKey, evidence);
    }
    if auth
        .as_ref()
        .and_then(|value| value.get("auth_mode"))
        .and_then(Value::as_str)
        == Some("chatgpt")
    {
        evidence.push("auth_mode_chatgpt".to_string());
        return (AuthMode::Subscription, evidence);
    }
    (AuthMode::Unknown, evidence)
}

fn codex_proxied(layout: &TakeoverLayout, mode: AuthMode) -> bool {
    let Ok(text) = fs::read_to_string(layout.codex_config()) else {
        return false;
    };
    let Ok(doc) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    let Some(provider_id) = doc.get("model_provider").and_then(toml::Value::as_str) else {
        return false;
    };
    if provider_id == ROUTER_PROVIDER_ID {
        return matches!(mode, AuthMode::ApiKey | AuthMode::Unknown);
    }
    if mode != AuthMode::Subscription || !is_custom_codex_model_provider_id(provider_id) {
        return false;
    }
    let Some(table) = doc
        .get("model_providers")
        .and_then(|providers| providers.get(provider_id))
    else {
        return false;
    };
    let requires_auth = table
        .get("requires_openai_auth")
        .and_then(toml::Value::as_bool);
    let websockets = table
        .get("supports_websockets")
        .and_then(toml::Value::as_bool);
    let base_url = table
        .get("base_url")
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    requires_auth == Some(true) && websockets == Some(false) && is_loopback_url(base_url)
}

fn claude_base_url(layout: &TakeoverLayout) -> Option<String> {
    match read_json_optional(&layout.claude_settings()) {
        JsonRead::Value(value) => value
            .pointer("/env/ANTHROPIC_BASE_URL")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

fn write_claude_base_url(layout: &TakeoverLayout, _subscription: bool) -> Result<(), AppError> {
    let settings = layout.claude_settings();
    let target = format!("http://127.0.0.1:{}", layout.router_port);
    if claude_base_url(layout).is_some_and(|url| urls_match(&url, &target)) {
        return Ok(());
    }
    ensure_claude_backup(&settings)?;
    let mut root = match read_json_optional(&settings) {
        JsonRead::Value(value) => value,
        JsonRead::Missing => json!({}),
        JsonRead::Unreadable => return Err(config_write_failed()),
    };
    if !root.is_object() {
        return Err(config_write_failed());
    }
    let env = root
        .as_object_mut()
        .ok_or_else(config_write_failed)?
        .entry("env")
        .or_insert_with(|| json!({}));
    if !env.is_object() {
        return Err(config_write_failed());
    }
    env.as_object_mut()
        .ok_or_else(config_write_failed)?
        .insert("ANTHROPIC_BASE_URL".to_string(), Value::String(target));
    let text = serde_json::to_string_pretty(&root).map_err(|_| config_write_failed())?;
    atomic_write(&settings, text.as_bytes())
}

fn ensure_claude_backup(settings: &Path) -> Result<(), AppError> {
    let backup = sibling(settings, BACKUP_SUFFIX);
    let absent = sibling(settings, ABSENT_SUFFIX);
    if backup.exists() || absent.exists() {
        return Ok(());
    }
    if settings.exists() {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(|_| config_write_failed())?;
        }
        fs::copy(settings, &backup).map_err(|_| config_write_failed())?;
    } else {
        if let Some(parent) = absent.parent() {
            fs::create_dir_all(parent).map_err(|_| config_write_failed())?;
        }
        fs::write(&absent, b"").map_err(|_| config_write_failed())?;
    }
    Ok(())
}

fn restore_claude_settings(settings: &Path) -> Result<(), AppError> {
    let backup = sibling(settings, BACKUP_SUFFIX);
    let absent = sibling(settings, ABSENT_SUFFIX);
    if absent.exists() {
        if settings.exists() {
            fs::remove_file(settings).map_err(|_| config_write_failed())?;
        }
        fs::remove_file(&absent).map_err(|_| config_write_failed())?;
        return Ok(());
    }
    if backup.exists() {
        atomic_write(
            settings,
            &fs::read(&backup).map_err(|_| config_write_failed())?,
        )?;
        fs::remove_file(&backup).map_err(|_| config_write_failed())?;
    }
    Ok(())
}

fn codex_points_at_router(config: &Path, port: u16) -> bool {
    let Ok(text) = fs::read_to_string(config) else {
        return false;
    };
    let Ok(doc) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    if doc.get("model_provider").and_then(toml::Value::as_str) != Some(ROUTER_PROVIDER_ID) {
        return false;
    }
    let expected = format!("http://127.0.0.1:{port}/v1");
    doc.get("model_providers")
        .and_then(|providers| providers.get(ROUTER_PROVIDER_ID))
        .and_then(|provider| provider.get("base_url"))
        .and_then(toml::Value::as_str)
        .is_some_and(|url| urls_match(url, &expected))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| config_write_failed())?;
    }
    let tmp = sibling(path, ".lub-takeover-tmp");
    fs::write(&tmp, bytes).map_err(|_| config_write_failed())?;
    fs::rename(&tmp, path).map_err(|_| {
        let _ = fs::remove_file(&tmp);
        config_write_failed()
    })
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    path.with_file_name(format!("{name}{suffix}"))
}

fn urls_match(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url
        .trim()
        .strip_prefix("http://")
        .or_else(|| url.trim().strip_prefix("https://"))
    else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.eq_ignore_ascii_case("127.0.0.1") || host.eq_ignore_ascii_case("localhost")
}

fn nonempty(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|text| !text.trim().is_empty())
}

fn nonempty_at(value: &Value, path: &[&str]) -> bool {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return false,
        }
    }
    current.as_str().is_some_and(|text| !text.trim().is_empty())
}

enum JsonRead {
    Missing,
    Value(Value),
    Unreadable,
}

fn read_json_optional(path: &Path) -> JsonRead {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => JsonRead::Value(json!({})),
        Ok(text) => serde_json::from_str(&text)
            .map(JsonRead::Value)
            .unwrap_or(JsonRead::Unreadable),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => JsonRead::Missing,
        Err(_) => JsonRead::Unreadable,
    }
}

fn terms_required() -> AppError {
    AppError::coded("TERMS_ACK_REQUIRED", "Terms acknowledgement is required.")
}

fn unsupported_tool() -> AppError {
    AppError::coded("UNSUPPORTED_TOOL", "This tool cannot be changed.")
}

fn config_write_failed() -> AppError {
    AppError::coded(
        "CONFIG_WRITE_FAILED",
        "Could not update the tool configuration.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn layout(home: &Path) -> TakeoverLayout {
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        TakeoverLayout {
            home: home.to_path_buf(),
            bin_dirs: vec![bin],
            router_port: 8788,
            env: DetectEnv::default(),
        }
    }

    fn scratch() -> tempfile::TempDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        tempfile::TempDir::with_prefix(format!("lub-takeover-{n}-")).unwrap()
    }

    fn assert_note_codes(codes: &[String]) {
        for code in codes {
            assert!(
                !code.is_empty()
                    && code
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "{code}"
            );
        }
    }

    #[test]
    fn unreadable_claude_auth_is_unknown_without_an_error() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude.json"), b"{").unwrap();
        let view = detect_claude_auth_at(&layout(home));
        assert_eq!(view.mode, AuthMode::Unknown);
        assert!(view
            .evidence
            .iter()
            .any(|code| code == "account_unreadable"));
        assert_note_codes(&view.evidence);
    }

    #[test]
    fn api_key_beats_subscription_evidence_and_hides_the_secret() {
        let dir = scratch();
        let home = dir.path();
        fs::write(
            home.join(".claude.json"),
            r#"{"oauthAccount":{"organizationType":"claude_pro"}}"#,
        )
        .unwrap();
        let mut layout = layout(home);
        layout.env.anthropic_api_key = Some("sk-secret".to_string());
        let view = detect_claude_auth_at(&layout);
        assert_eq!(view.mode, AuthMode::ApiKey);
        assert!(view
            .evidence
            .iter()
            .any(|code| code == "process_env_api_key"));
        assert!(!view.evidence.iter().any(|code| code.contains("sk-secret")));
    }

    #[test]
    fn subscription_apply_writes_nothing_and_requires_terms() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude")).unwrap();
        let original = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":""}}"#;
        fs::write(home.join(".claude/settings.json"), original).unwrap();
        fs::write(
            home.join(".claude.json"),
            r#"{"oauthAccount":{"organizationType":"claude_max"}}"#,
        )
        .unwrap();
        let layout = layout(home);
        let error = apply_takeover_at(&layout, "claude", true).unwrap_err();
        assert_eq!(error.code(), Some("TERMS_ACK_REQUIRED"));
        assert_eq!(
            fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            original
        );
        let disable = apply_takeover_at(&layout, "claude", false).unwrap_err();
        assert_eq!(disable.code(), Some("TERMS_ACK_REQUIRED"));
        assert!(!home
            .join(".claude/settings.json.lub-takeover-backup")
            .exists());
    }

    #[test]
    fn subscription_proxy_requires_ack_then_restores() {
        let dir = scratch();
        let home = dir.path();
        let original = "{\"theme\":\"dark\"}\n";
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/settings.json"), original).unwrap();
        let layout = layout(home);
        let rejected = set_claude_subscription_proxy_at(&layout, true, None).unwrap_err();
        assert_eq!(rejected.code(), Some("TERMS_ACK_REQUIRED"));
        assert_eq!(
            fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            original
        );

        set_claude_subscription_proxy_at(&layout, true, Some(1_700_000_000)).unwrap();
        let written = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(written.contains("http://127.0.0.1:8788"));
        assert!(written.contains("dark"));
        let backup =
            fs::read_to_string(home.join(".claude/settings.json.lub-takeover-backup")).unwrap();
        assert_eq!(backup, original);
        set_claude_subscription_proxy_at(&layout, true, Some(1_700_000_001)).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(".claude/settings.json.lub-takeover-backup")).unwrap(),
            original
        );

        set_claude_subscription_proxy_at(&layout, false, None).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            original
        );
        assert!(!home
            .join(".claude/settings.json.lub-takeover-backup")
            .exists());
    }

    #[test]
    fn api_key_takeover_is_idempotent_and_disable_restores_a_missing_file() {
        let dir = scratch();
        let home = dir.path();
        let mut layout = layout(home);
        layout.env.anthropic_auth_token = Some("token".to_string());
        fs::write(layout.bin_dirs[0].join("claude"), b"bin").unwrap();
        apply_takeover_at(&layout, "claude", true).unwrap();
        let created = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        apply_takeover_at(&layout, "claude", true).unwrap();
        assert_eq!(
            fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            created
        );
        assert!(home
            .join(".claude/settings.json.lub-takeover-absent")
            .exists());
        apply_takeover_at(&layout, "claude", false).unwrap();
        assert!(!home.join(".claude/settings.json").exists());
    }

    #[test]
    fn unknown_enable_skips_the_write() {
        let dir = scratch();
        let home = dir.path();
        let layout = layout(home);
        let result = apply_takeover_at(&layout, "claude", true).unwrap();
        assert_eq!(result.note_codes, vec!["auth_unknown_skipped".to_string()]);
        assert!(!home.join(".claude/settings.json").exists());
        assert_note_codes(&result.note_codes);
    }

    #[test]
    fn cursor_apply_is_unsupported_and_detect_returns_codes() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".cursor")).unwrap();
        let layout = layout(home);
        let error = apply_takeover_at(&layout, "cursor", true).unwrap_err();
        assert_eq!(error.code(), Some("UNSUPPORTED_TOOL"));
        let rows = detect_tools_at(&layout);
        assert!(rows.iter().all(|row| row.version.is_none()));
        let cursor = rows.iter().find(|row| row.tool == "cursor").unwrap();
        assert!(cursor.installed);
        assert_eq!(cursor.auth_mode, AuthMode::NotApplicable);
        assert!(!cursor.enforceable);
        assert!(!cursor.proxied);
        for row in &rows {
            assert_note_codes(&row.note_codes);
            let text = serde_json::to_string(row).unwrap();
            assert!(!text.contains("http"));
        }
        let serialized = serde_json::to_value(cursor).unwrap();
        assert_eq!(serialized["authMode"], "n_a");
    }

    #[test]
    fn codex_subscription_does_not_reuse_the_api_key_pointer() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let original = "\
model_provider = \"openai_https\"\n\
\n\
[model_providers.openai_https]\n\
requires_openai_auth = true\n\
supports_websockets = false\n\
base_url = \"https://chatgpt.com/backend-api/codex\"\n";
        fs::write(home.join(".codex/config.toml"), original).unwrap();
        fs::write(
            home.join(".codex/auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"secret-token"}}"#,
        )
        .unwrap();
        let layout = layout(home);
        let error = apply_takeover_at(&layout, "codex", true).unwrap_err();
        assert_eq!(error.code(), Some("TERMS_ACK_REQUIRED"));
        assert_eq!(
            fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
            original
        );
        let row = detect_tools_at(&layout)
            .into_iter()
            .find(|row| row.tool == "codex")
            .unwrap();
        assert_eq!(row.auth_mode, AuthMode::Subscription);
        assert!(!row.proxied);
        assert!(!row.enforceable);
        assert!(!serde_json::to_string(&row)
            .unwrap()
            .contains("secret-token"));
    }

    #[test]
    fn codex_subscription_is_proxied_only_for_the_explicit_http_triple() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"x"}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "\
model_provider = \"openai_https\"\n\
\n\
[model_providers.openai_https]\n\
requires_openai_auth = true\n\
supports_websockets = false\n\
base_url = \"http://127.0.0.1:8788/v1\"\n",
        )
        .unwrap();
        let layout = layout(home);
        let row = detect_tools_at(&layout)
            .into_iter()
            .find(|row| row.tool == "codex")
            .unwrap();
        assert!(row.proxied, "{row:?}");

        fs::write(
            home.join(".codex/config.toml"),
            "\
model_provider = \"openai_https\"\n\
\n\
[model_providers.openai_https]\n\
requires_openai_auth = true\n\
base_url = \"http://127.0.0.1:8788/v1\"\n",
        )
        .unwrap();
        let missing_flag = detect_tools_at(&layout)
            .into_iter()
            .find(|row| row.tool == "codex")
            .unwrap()
            .proxied;
        assert!(!missing_flag);
    }

    #[test]
    fn codex_api_key_pointer_is_idempotent_and_restores() {
        let dir = scratch();
        let home = dir.path();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let original = "model_provider = \"openai\"\n";
        fs::write(home.join(".codex/config.toml"), original).unwrap();
        fs::write(
            home.join(".codex/auth.json"),
            r#"{"OPENAI_API_KEY":"sk-test"}"#,
        )
        .unwrap();
        let layout = layout(home);
        apply_takeover_at(&layout, "codex", true).unwrap();
        let backups = fs::read_dir(home.join(".codex"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("before-router")
            })
            .count();
        assert_eq!(backups, 1);
        apply_takeover_at(&layout, "codex", true).unwrap();
        let backups_again = fs::read_dir(home.join(".codex"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("before-router")
            })
            .count();
        assert_eq!(backups_again, 1);
        apply_takeover_at(&layout, "codex", false).unwrap();
        let restored = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        let selection = restored
            .lines()
            .find(|line| line.trim_start().starts_with("model_provider"))
            .unwrap_or("");
        assert!(selection.contains("openai"), "{restored}");
        assert!(!selection.contains(ROUTER_PROVIDER_ID), "{restored}");
    }

    #[test]
    fn windows_stub_does_not_claim_a_version() {
        let rows = windows_tool_stub();
        assert!(rows.iter().all(|row| row.version.is_none() && !row.proxied));
        assert!(rows
            .iter()
            .all(|row| row.note_codes == ["windows_detection_pending"]));
    }
}
