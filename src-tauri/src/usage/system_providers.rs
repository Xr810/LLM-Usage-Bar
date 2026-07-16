use super::domain::{BillingKind, SystemProviderAuthKind, TokenSource};

pub const CHATGPT_SUBSCRIPTION_ID: &str = "system-chatgpt-subscription";
pub const CLAUDE_SUBSCRIPTION_ID: &str = "system-claude-subscription";
pub const OPENAI_API_ID: &str = "system-openai-api";
pub const ANTHROPIC_API_ID: &str = "system-anthropic-api";
pub const OPENROUTER_API_ID: &str = "system-openrouter-api";
pub const MANAGED_CODEX_QUOTA_SOURCE: &str = "codex_oauth";

pub struct SystemProviderDefinition {
    pub id: &'static str,
    pub preset_key: &'static str,
    pub name: &'static str,
    pub billing_kind: BillingKind,
    pub product_group_id: &'static str,
    pub token_sources: &'static [TokenSource],
    pub auth_kind: SystemProviderAuthKind,
    pub quota_source: Option<&'static str>,
    pub quota_interval_seconds: Option<u64>,
    pub upstream_protocol: Option<&'static str>,
    pub route_config: Option<serde_json::Value>,
}

pub fn system_provider_definitions() -> Vec<SystemProviderDefinition> {
    vec![
        SystemProviderDefinition {
            id: CHATGPT_SUBSCRIPTION_ID,
            preset_key: "chatgpt-subscription",
            name: "ChatGPT Plus/Pro",
            billing_kind: BillingKind::Subscription,
            product_group_id: "chatgpt-subscription",
            token_sources: &[TokenSource::Proxy, TokenSource::SessionLog],
            auth_kind: SystemProviderAuthKind::CodexOauth,
            quota_source: Some(MANAGED_CODEX_QUOTA_SOURCE),
            quota_interval_seconds: Some(300),
            upstream_protocol: Some("codex"),
            route_config: None,
        },
        SystemProviderDefinition {
            id: CLAUDE_SUBSCRIPTION_ID,
            preset_key: "claude-subscription",
            name: "Claude Pro/Max",
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude-subscription",
            token_sources: &[TokenSource::SessionLog],
            auth_kind: SystemProviderAuthKind::ClaudeCli,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: None,
            route_config: None,
        },
        SystemProviderDefinition {
            id: OPENAI_API_ID,
            preset_key: "openai-api",
            name: "OpenAI API",
            billing_kind: BillingKind::Metered,
            product_group_id: "openai-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("codex"),
            route_config: Some(serde_json::json!({
                "base_url": "https://api.openai.com/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
        SystemProviderDefinition {
            id: ANTHROPIC_API_ID,
            preset_key: "anthropic-api",
            name: "Anthropic API",
            billing_kind: BillingKind::Metered,
            product_group_id: "anthropic-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("claude"),
            route_config: Some(serde_json::json!({
                "base_url": "https://api.anthropic.com",
                "apiFormat": "anthropic",
                "authMode": "x_api_key"
            })),
        },
        SystemProviderDefinition {
            id: OPENROUTER_API_ID,
            preset_key: "openrouter-api",
            name: "OpenRouter",
            billing_kind: BillingKind::Metered,
            product_group_id: "openrouter-api",
            token_sources: &[TokenSource::Proxy],
            auth_kind: SystemProviderAuthKind::ProviderApiKey,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("codex"),
            route_config: Some(serde_json::json!({
                "base_url": "https://openrouter.ai/api/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
    ]
}

pub(crate) fn system_binding_route_protocol(
    preset_key: &str,
    agent_module_id: &str,
) -> Option<Option<&'static str>> {
    match (preset_key, agent_module_id) {
        ("chatgpt-subscription", "codex") => Some(Some("codex")),
        ("claude-subscription", "claude-code") => Some(None),
        ("openai-api", "codex") => Some(Some("codex")),
        ("openai-api", "opencode") => Some(Some("opencode")),
        ("openai-api", "openclaw") => Some(Some("openclaw")),
        ("openai-api", "hermes") => Some(Some("hermes")),
        ("anthropic-api", "claude-code") => Some(Some("claude")),
        ("openrouter-api", "claude-code") => Some(Some("claude")),
        ("openrouter-api", "codex") => Some(Some("codex")),
        ("openrouter-api", "opencode") => Some(Some("opencode")),
        ("openrouter-api", "openclaw") => Some(Some("openclaw")),
        ("openrouter-api", "hermes") => Some(Some("hermes")),
        _ => None,
    }
}

pub(crate) fn is_fixed_api_preset(preset_key: Option<&str>) -> bool {
    matches!(
        preset_key,
        Some("openai-api" | "anthropic-api" | "openrouter-api")
    )
}

#[cfg(test)]
mod tests {
    use super::system_binding_route_protocol;

    #[test]
    fn system_binding_protocol_matrix_is_exact_and_server_owned() {
        let supported = [
            ("chatgpt-subscription", "codex", Some("codex")),
            ("claude-subscription", "claude-code", None),
            ("openai-api", "codex", Some("codex")),
            ("openai-api", "opencode", Some("opencode")),
            ("openai-api", "openclaw", Some("openclaw")),
            ("openai-api", "hermes", Some("hermes")),
            ("anthropic-api", "claude-code", Some("claude")),
            ("openrouter-api", "claude-code", Some("claude")),
            ("openrouter-api", "codex", Some("codex")),
            ("openrouter-api", "opencode", Some("opencode")),
            ("openrouter-api", "openclaw", Some("openclaw")),
            ("openrouter-api", "hermes", Some("hermes")),
        ];
        for (preset, agent, protocol) in supported {
            assert_eq!(
                system_binding_route_protocol(preset, agent),
                Some(protocol),
                "{preset}/{agent}"
            );
        }
        for (preset, agent) in [
            ("chatgpt-subscription", "claude-code"),
            ("claude-subscription", "codex"),
            ("openai-api", "claude-code"),
            ("anthropic-api", "codex"),
            ("openrouter-api", "unknown-agent"),
            ("unknown-provider", "codex"),
        ] {
            assert_eq!(system_binding_route_protocol(preset, agent), None);
        }
    }
}
