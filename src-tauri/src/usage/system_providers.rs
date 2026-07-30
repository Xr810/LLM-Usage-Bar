use super::domain::{BillingKind, SystemProviderAuthKind, TokenSource};

pub const CHATGPT_SUBSCRIPTION_ID: &str = "system-chatgpt-subscription";
pub const CLAUDE_SUBSCRIPTION_ID: &str = "system-claude-subscription";
pub const OPENAI_API_ID: &str = "system-openai-api";
pub const ANTHROPIC_API_ID: &str = "system-anthropic-api";
pub const OPENROUTER_API_ID: &str = "system-openrouter-api";
pub const GEMINI_API_ID: &str = "system-gemini-api";
pub const XAI_API_ID: &str = "system-xai-api";
pub const DEEPSEEK_API_ID: &str = "system-deepseek-api";
pub const KIMI_API_ID: &str = "system-kimi-api";
pub const GLM_API_ID: &str = "system-glm-api";
pub const QWEN_API_ID: &str = "system-qwen-api";
pub const MINIMAX_API_ID: &str = "system-minimax-api";
pub const MISTRAL_API_ID: &str = "system-mistral-api";
pub const GROQ_API_ID: &str = "system-groq-api";
pub const TOGETHER_API_ID: &str = "system-together-api";
pub const FIREWORKS_API_ID: &str = "system-fireworks-api";
pub const PERPLEXITY_API_ID: &str = "system-perplexity-api";
pub const SILICONFLOW_API_ID: &str = "system-siliconflow-api";
pub const NVIDIA_NIM_API_ID: &str = "system-nvidia-nim-api";
pub const CEREBRAS_API_ID: &str = "system-cerebras-api";
pub const MANAGED_CODEX_QUOTA_SOURCE: &str = "codex_oauth";
pub const CLAUDE_LOCAL_QUOTA_SOURCE: &str = "claude_local";

pub struct SystemProviderDefinition {
    pub id: &'static str,
    pub preset_key: &'static str,
    pub name: &'static str,
    pub billing_kind: BillingKind,
    pub product_group_id: &'static str,
    pub token_sources: &'static [TokenSource],
    pub auth_kind: SystemProviderAuthKind,
    pub default_enabled: bool,
    pub quota_source: Option<&'static str>,
    pub quota_interval_seconds: Option<u64>,
    pub upstream_protocol: Option<&'static str>,
    pub connection_test_path: Option<&'static str>,
    pub route_config: Option<serde_json::Value>,
}

fn metered_api_provider(
    id: &'static str,
    preset_key: &'static str,
    name: &'static str,
    base_url: &'static str,
) -> SystemProviderDefinition {
    metered_api_provider_with_connection_path(id, preset_key, name, base_url, "/models")
}

fn metered_api_provider_with_connection_path(
    id: &'static str,
    preset_key: &'static str,
    name: &'static str,
    base_url: &'static str,
    connection_test_path: &'static str,
) -> SystemProviderDefinition {
    SystemProviderDefinition {
        id,
        preset_key,
        name,
        billing_kind: BillingKind::Metered,
        product_group_id: preset_key,
        token_sources: &[TokenSource::Proxy],
        auth_kind: SystemProviderAuthKind::ProviderApiKey,
        default_enabled: false,
        quota_source: None,
        quota_interval_seconds: None,
        upstream_protocol: Some("codex"),
        connection_test_path: Some(connection_test_path),
        route_config: Some(serde_json::json!({
            "base_url": base_url,
            "apiFormat": "openai_chat",
            "authMode": "bearer"
        })),
    }
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
            default_enabled: true,
            quota_source: Some(MANAGED_CODEX_QUOTA_SOURCE),
            quota_interval_seconds: Some(300),
            upstream_protocol: Some("codex"),
            connection_test_path: None,
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
            default_enabled: true,
            quota_source: Some(CLAUDE_LOCAL_QUOTA_SOURCE),
            quota_interval_seconds: Some(300),
            upstream_protocol: None,
            connection_test_path: None,
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
            default_enabled: true,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("codex"),
            connection_test_path: Some("/models"),
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
            default_enabled: true,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("claude"),
            connection_test_path: Some("/v1/models"),
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
            default_enabled: true,
            quota_source: None,
            quota_interval_seconds: None,
            upstream_protocol: Some("codex"),
            // OpenRouter's model catalog is publicly readable, so it cannot
            // validate the submitted key. `/key` is the official read-only
            // endpoint for the current authenticated API key.
            connection_test_path: Some("/key"),
            route_config: Some(serde_json::json!({
                "base_url": "https://openrouter.ai/api/v1",
                "apiFormat": "openai_chat",
                "authMode": "bearer"
            })),
        },
        metered_api_provider(
            GEMINI_API_ID,
            "gemini-api",
            "Google Gemini API",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        ),
        metered_api_provider(
            XAI_API_ID,
            "xai-api",
            "xAI (Grok) API",
            "https://api.x.ai/v1",
        ),
        metered_api_provider(
            DEEPSEEK_API_ID,
            "deepseek-api",
            "DeepSeek API",
            "https://api.deepseek.com",
        ),
        metered_api_provider(
            KIMI_API_ID,
            "kimi-api",
            "Kimi / Moonshot API",
            "https://api.moonshot.cn/v1",
        ),
        metered_api_provider(
            GLM_API_ID,
            "glm-api",
            "GLM / Z.AI API",
            "https://api.z.ai/api/paas/v4",
        ),
        metered_api_provider(
            QWEN_API_ID,
            "qwen-api",
            "Qwen / DashScope API",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
        ),
        metered_api_provider(
            MINIMAX_API_ID,
            "minimax-api",
            "MiniMax API",
            "https://api.minimaxi.com/v1",
        ),
        metered_api_provider(
            MISTRAL_API_ID,
            "mistral-api",
            "Mistral AI API",
            "https://api.mistral.ai/v1",
        ),
        metered_api_provider(
            GROQ_API_ID,
            "groq-api",
            "Groq API",
            "https://api.groq.com/openai/v1",
        ),
        metered_api_provider(
            TOGETHER_API_ID,
            "together-api",
            "Together AI",
            "https://api.together.xyz/v1",
        ),
        metered_api_provider(
            FIREWORKS_API_ID,
            "fireworks-api",
            "Fireworks AI",
            "https://api.fireworks.ai/inference/v1",
        ),
        metered_api_provider_with_connection_path(
            PERPLEXITY_API_ID,
            "perplexity-api",
            "Perplexity API",
            "https://api.perplexity.ai",
            // `/v1/models` is public and would accept a bogus key. Listing
            // async Sonar requests is read-only and requires authentication.
            "/v1/async/sonar",
        ),
        metered_api_provider(
            SILICONFLOW_API_ID,
            "siliconflow-api",
            "SiliconFlow API",
            "https://api.siliconflow.cn/v1",
        ),
        metered_api_provider_with_connection_path(
            NVIDIA_NIM_API_ID,
            "nvidia-nim-api",
            "NVIDIA NIM API",
            "https://integrate.api.nvidia.com/v1",
            // NVIDIA's hosted `/models` catalog is public and does not prove
            // that a key can invoke hosted inference. Keep the canonical path
            // for discovery, but the UI suppresses the misleading key test.
            "/models",
        ),
        metered_api_provider(
            CEREBRAS_API_ID,
            "cerebras-api",
            "Cerebras API",
            "https://api.cerebras.ai/v1",
        ),
    ]
}

pub fn system_provider_definition(preset_key: &str) -> Option<SystemProviderDefinition> {
    system_provider_definitions()
        .into_iter()
        .find(|definition| definition.preset_key == preset_key)
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
    preset_key
        .and_then(system_provider_definition)
        .is_some_and(|definition| definition.auth_kind == SystemProviderAuthKind::ProviderApiKey)
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
