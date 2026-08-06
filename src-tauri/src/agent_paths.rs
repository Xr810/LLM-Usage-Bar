//! Filesystem locations of the CLI agents this app reads usage from.
//!
//! Only the paths survive: writing those tools' configs went with provider
//! switching, but the session-log parsers and quota collectors still need to
//! find their directories.

use crate::config::get_home_dir;
use crate::settings::{
    get_hermes_override_dir, get_openclaw_override_dir, get_opencode_override_dir,
};
use serde_json::Value;
use std::path::PathBuf;
use toml_edit::DocumentMut;

/// 获取 Codex 配置目录路径
pub fn get_codex_config_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_codex_override_dir() {
        return custom;
    }

    get_home_dir().join(".codex")
}
/// 获取 Codex auth.json 路径
pub fn get_codex_auth_path() -> PathBuf {
    get_codex_config_dir().join("auth.json")
}
/// 获取 Gemini 配置目录路径（支持设置覆盖）
pub fn get_gemini_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_gemini_override_dir() {
        return custom;
    }

    get_home_dir().join(".gemini")
}
/// 获取 OpenCode SQLite 数据库路径
/// 优先级: OPENCODE_DB 环境变量 > XDG_DATA_HOME > ~/.local/share/opencode
pub fn get_opencode_db_path() -> PathBuf {
    // 支持 OPENCODE_DB 环境变量覆盖（忽略空字符串）
    if let Ok(custom_path) = std::env::var("OPENCODE_DB") {
        if !custom_path.is_empty() {
            let path = PathBuf::from(&custom_path);
            if path.is_absolute() {
                return path;
            }
            // 相对路径基于数据目录
            return get_opencode_data_dir().join(path);
        }
    }

    get_opencode_data_dir().join("opencode.db")
}

fn get_opencode_data_dir() -> PathBuf {
    // 尊重 XDG_DATA_HOME（按 XDG 规范，空字符串视为未设置）
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            return PathBuf::from(xdg_data).join("opencode");
        }
    }

    // OpenCode 使用 xdg-basedir，不遵守 macOS/Windows 平台约定，
    // 所有平台默认都落在 ~/.local/share/opencode
    crate::config::get_home_dir()
        .join(".local")
        .join("share")
        .join("opencode")
}

/// 获取 Hermes 配置目录
///
/// 解析顺序对齐 Hermes 自身的 `get_hermes_home()`:
///   1. CCS 设置 `hermes_config_dir`(显式覆盖)
///   2. `HERMES_HOME` 环境变量(trim 后非空;按原样,不展开 `~`,与 Hermes `Path(val)` 一致)
///   3. 平台默认(Windows: `%LOCALAPPDATA%\hermes`,Mac/Linux: `~/.hermes`)
pub fn get_hermes_dir() -> PathBuf {
    if let Some(override_dir) = get_hermes_override_dir() {
        return override_dir;
    }

    if let Some(raw) = std::env::var_os("HERMES_HOME") {
        let value = raw.to_string_lossy();
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    default_hermes_dir()
}

pub fn extract_codex_api_key(auth: Option<&Value>, config_text: Option<&str>) -> Option<String> {
    auth.and_then(extract_codex_auth_api_key)
        .or_else(|| config_text.and_then(extract_codex_experimental_bearer_token))
}

/// Extract the upstream base URL from a Codex `config.toml` string.
///
/// Prefers the active `[model_providers.<model_provider>].base_url`, falling
/// back to a top-level `base_url`. Deliberately never reads a non-active
/// `[model_providers.*]` section — the frontend `extractCodexBaseUrl`
/// (`getRecoverableBaseUrlAssignments`) excludes those too, and a leftover
/// section unrelated to the active provider must not leak into `{{baseUrl}}`.
pub fn extract_codex_base_url(config_text: &str) -> Option<String> {
    let doc = toml::from_str::<toml::Value>(config_text).ok()?;

    if let Some(active_provider) = doc.get("model_provider").and_then(|v| v.as_str()) {
        if let Some(base_url) = doc
            .get("model_providers")
            .and_then(|providers| providers.get(active_provider))
            .and_then(|provider| provider.get("base_url"))
            .and_then(|v| v.as_str())
        {
            return Some(base_url.to_string());
        }
    }

    doc.get("base_url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

pub fn get_opencode_dir() -> PathBuf {
    if let Some(override_dir) = get_opencode_override_dir() {
        return override_dir;
    }

    crate::config::get_home_dir()
        .join(".config")
        .join("opencode")
}

/// 平台默认 Hermes 目录(Windows):对齐 Hermes `_get_platform_default_hermes_home()`——
/// 读 `LOCALAPPDATA` 环境变量,缺失/空时回退 `~\AppData\Local`,再拼 `hermes`。
#[cfg(target_os = "windows")]
fn default_hermes_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::get_home_dir().join("AppData").join("Local"))
        .join("hermes")
}

/// 平台默认 Hermes 目录(Mac/Linux):`~/.hermes`。
#[cfg(not(target_os = "windows"))]
fn default_hermes_dir() -> PathBuf {
    crate::config::get_home_dir().join(".hermes")
}

pub fn extract_codex_auth_api_key(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

/// Extract a provider-scoped `experimental_bearer_token` from Codex `config.toml`.
///
/// Mobile compat: third-party providers may store the API key inside
/// `[model_providers.<id>].experimental_bearer_token` while keeping the
/// user's ChatGPT login cache intact in `auth.json`. Falls back to the
/// top-level `experimental_bearer_token` when no active model provider is set.
pub fn extract_codex_experimental_bearer_token(config_text: &str) -> Option<String> {
    if !config_text.contains("experimental_bearer_token") {
        return None;
    }
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let provider_id = active_codex_model_provider_id(&doc);

    let top_level_token = || {
        doc.get("experimental_bearer_token")
            .and_then(|item| item.as_str())
    };
    let token = match provider_id.as_deref() {
        Some(id) if is_custom_codex_model_provider_id(id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|table| table.get(id))
            .and_then(|item| item.as_table())
            .and_then(|table| table.get("experimental_bearer_token"))
            .and_then(|item| item.as_str())
            .or_else(top_level_token),
        Some(_) => top_level_token(),
        None => top_level_token(),
    };

    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn active_codex_model_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub(crate) fn is_custom_codex_model_provider_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && !CODEX_RESERVED_MODEL_PROVIDER_IDS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(id))
}

/// Reserved built-in provider IDs from OpenAI Codex's config/model-provider
/// catalog. Keep in sync with Codex `RESERVED_MODEL_PROVIDER_IDS` and legacy
/// removed provider aliases.
const CODEX_RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "openai",
    "ollama",
    "lmstudio",
    "oss",
    "ollama-chat",
];

pub fn get_openclaw_dir() -> PathBuf {
    if let Some(override_dir) = get_openclaw_override_dir() {
        return override_dir;
    }

    crate::config::get_home_dir().join(".openclaw")
}
