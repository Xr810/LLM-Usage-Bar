//! 官方供应商种子数据
//!
//! 启动时调用 `Database::init_default_official_providers` 把这些条目
//! 写入 `providers` 表，让所有用户都能看到一个"一键切回官方"的入口。
//!
//! **这里是数据，不是 core 认识 provider**：下面出现的任何一家名字都只是
//! 目录里的行数据，没有任何逻辑代码按它们分支。加一家新 agent / 新 provider，
//! 在这张表里加一行即可，不需要改任何逻辑代码。
//!
//! 字段与前端预设保持一致，参见：
//! - `src/config/claudeProviderPresets.ts`（"Claude Official"）
//! - `src/config/codexProviderPresets.ts`（"OpenAI Official"）
//! - `src/config/geminiProviderPresets.ts`（"Google Official"）

use crate::app_config::AppType;

pub(crate) const CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID: &str = "claude-desktop-official";

/// 单条官方供应商种子定义。
pub(crate) struct OfficialProviderSeed {
    pub id: &'static str,
    pub app_type: AppType,
    pub name: &'static str,
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    /// settings_config 的 JSON 字符串，每个 app 结构不同。
    pub settings_config_json: &'static str,
}

/// 内置 provider 目录：Claude / Claude Desktop / Codex / Gemini 的官方预设。
///
/// id 固定，便于幂等检查；name 直接用英文原名（与前端预设一致），不做 i18n。
/// 这是纯数据表：加一家 = 加一行，core 不感知任何一家的存在。
pub(crate) const BUILTIN_PROVIDER_CATALOG: &[OfficialProviderSeed] = &[
    OfficialProviderSeed {
        id: "claude-official",
        app_type: AppType::Claude,
        name: "Claude Official",
        website_url: "https://www.anthropic.com/claude-code",
        icon: "anthropic",
        icon_color: "#D4915D",
        // 空 env 让用户走 Claude CLI 默认认证流程
        settings_config_json: r#"{"env":{}}"#,
    },
    OfficialProviderSeed {
        id: CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID,
        app_type: AppType::ClaudeDesktop,
        name: "Claude Desktop Official",
        website_url: "https://claude.ai/download",
        icon: "anthropic",
        icon_color: "#D4915D",
        // 空 env 只是占位；切换该 provider 时会恢复 Claude Desktop 1P 模式
        settings_config_json: r#"{"env":{}}"#,
    },
    OfficialProviderSeed {
        id: "codex-official",
        app_type: AppType::Codex,
        name: "OpenAI Official",
        website_url: "https://chatgpt.com/codex",
        icon: "openai",
        icon_color: "#00A67E",
        // 空 auth + 空 config 让用户走 ChatGPT Plus/Pro OAuth
        settings_config_json: r#"{"auth":{},"config":""}"#,
    },
    OfficialProviderSeed {
        id: "gemini-official",
        app_type: AppType::Gemini,
        name: "Google Official",
        website_url: "https://ai.google.dev/",
        icon: "gemini",
        icon_color: "#4285F4",
        // 空 env + 空 config 让用户走 Google OAuth
        settings_config_json: r#"{"env":{},"config":{}}"#,
    },
];

/// 判断给定的 provider id 是否属于内置目录。
///
/// 单一事实源：直接扫描 `BUILTIN_PROVIDER_CATALOG`，避免在多处重复维护 id 列表。
pub(crate) fn is_official_seed_id(id: &str) -> bool {
    BUILTIN_PROVIDER_CATALOG.iter().any(|seed| seed.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_seeds_include_claude_desktop() {
        let seed = BUILTIN_PROVIDER_CATALOG
            .iter()
            .find(|seed| seed.id == CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID)
            .expect("claude desktop official seed");

        assert_eq!(seed.app_type, AppType::ClaudeDesktop);
        assert!(is_official_seed_id(CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID));
    }

    /// 内置目录是纯数据：重构只允许改它的名字/注释，内容必须与重构前逐项相同。
    #[test]
    fn builtin_provider_catalog_contents_are_unchanged() {
        assert_eq!(BUILTIN_PROVIDER_CATALOG.len(), 4);

        let ids: Vec<&str> = BUILTIN_PROVIDER_CATALOG
            .iter()
            .map(|seed| seed.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "claude-official",
                CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID,
                "codex-official",
                "gemini-official"
            ]
        );

        let seed = |id: &str| {
            BUILTIN_PROVIDER_CATALOG
                .iter()
                .find(|seed| seed.id == id)
                .unwrap_or_else(|| panic!("missing builtin seed: {id}"))
        };

        let claude = seed("claude-official");
        assert_eq!(claude.app_type, AppType::Claude);
        assert_eq!(claude.name, "Claude Official");
        assert_eq!(claude.website_url, "https://www.anthropic.com/claude-code");
        assert_eq!(claude.icon, "anthropic");
        assert_eq!(claude.icon_color, "#D4915D");
        assert_eq!(claude.settings_config_json, r#"{"env":{}}"#);

        let claude_desktop = seed(CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID);
        assert_eq!(claude_desktop.app_type, AppType::ClaudeDesktop);
        assert_eq!(claude_desktop.name, "Claude Desktop Official");
        assert_eq!(claude_desktop.website_url, "https://claude.ai/download");
        assert_eq!(claude_desktop.icon, "anthropic");
        assert_eq!(claude_desktop.icon_color, "#D4915D");
        assert_eq!(claude_desktop.settings_config_json, r#"{"env":{}}"#);

        let codex = seed("codex-official");
        assert_eq!(codex.app_type, AppType::Codex);
        assert_eq!(codex.name, "OpenAI Official");
        assert_eq!(codex.website_url, "https://chatgpt.com/codex");
        assert_eq!(codex.icon, "openai");
        assert_eq!(codex.icon_color, "#00A67E");
        assert_eq!(codex.settings_config_json, r#"{"auth":{},"config":""}"#);

        let gemini = seed("gemini-official");
        assert_eq!(gemini.app_type, AppType::Gemini);
        assert_eq!(gemini.name, "Google Official");
        assert_eq!(gemini.website_url, "https://ai.google.dev/");
        assert_eq!(gemini.icon, "gemini");
        assert_eq!(gemini.icon_color, "#4285F4");
        assert_eq!(gemini.settings_config_json, r#"{"env":{},"config":{}}"#);
    }
}
