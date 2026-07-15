use super::domain::{BillingKind, SystemProviderAuthKind, TokenSource};

pub const CHATGPT_SUBSCRIPTION_ID: &str = "system-chatgpt-subscription";
pub const CLAUDE_SUBSCRIPTION_ID: &str = "system-claude-subscription";
pub const OPENAI_API_ID: &str = "system-openai-api";
pub const ANTHROPIC_API_ID: &str = "system-anthropic-api";
pub const OPENROUTER_API_ID: &str = "system-openrouter-api";

pub struct SystemProviderDefinition {
    pub id: &'static str,
    pub preset_key: &'static str,
    pub name: &'static str,
    pub billing_kind: BillingKind,
    pub product_group_id: &'static str,
    pub token_sources: &'static [TokenSource],
    pub auth_kind: SystemProviderAuthKind,
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
            upstream_protocol: Some("codex"),
            route_config: Some(serde_json::json!({
                "base_url": "https://openrouter.ai/api/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
    ]
}
