use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingKind {
    Subscription,
    Metered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    Proxy,
    SessionLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemProviderAuthKind {
    CodexOauth,
    ClaudeCli,
    ProviderApiKey,
}

pub const CLAUDE_CODE_AGENT_MODULE_ID: &str = "claude-code";
pub const CODEX_AGENT_MODULE_ID: &str = "codex";

/// Return the immutable Agent owner for a trusted local session parser.
/// Provider selection remains configurable, but parser identity does not.
pub fn session_agent_module_id(source: &str) -> Option<&'static str> {
    match source {
        "claude" => Some(CLAUDE_CODE_AGENT_MODULE_ID),
        "codex" => Some(CODEX_AGENT_MODULE_ID),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    Upstream,
    Estimated,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModuleInput {
    pub id: Option<String>,
    pub name: String,
    pub sort_order: i64,
    pub visible: bool,
}

impl<'de> Deserialize<'de> for AgentModuleInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WireInput {
            id: Option<String>,
            name: String,
            sort_order: i64,
            visible: bool,
        }

        let input = WireInput::deserialize(deserializer)
            .map_err(|_| de::Error::custom("invalid_agent_module"))?;
        if input.id.as_deref() == Some("api") {
            return Err(de::Error::custom("invalid_agent_module"));
        }
        Ok(Self {
            id: input.id,
            name: input.name,
            sort_order: input.sort_order,
            visible: input.visible,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModuleView {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
    pub visible: bool,
    pub is_fixed: bool,
    pub archived_at: Option<i64>,
    pub provider_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderBindingInput {
    pub id: Option<String>,
    pub agent_module_id: String,
    pub provider_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingCredentialStatus {
    NotRequired,
    Missing,
    Configured,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderBindingView {
    pub id: String,
    pub agent_module_id: String,
    pub provider_id: String,
    pub enabled: bool,
    pub effective_enabled: bool,
    pub credential_status: BindingCredentialStatus,
    pub can_clear_credential: bool,
    pub credential_version: u64,
    pub route_protocol: Option<String>,
    pub local_credential_status: BindingCredentialStatus,
    pub provider_credential_status: BindingCredentialStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBindingKeyReveal {
    pub binding_id: String,
    pub credential_version: u64,
    pub local_key: String,
}

impl std::fmt::Debug for LocalBindingKeyReveal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalBindingKeyReveal([REDACTED])")
    }
}

impl Drop for LocalBindingKeyReveal {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.local_key.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProxyRouteSetup {
    pub binding_id: String,
    pub provider_id: String,
    pub protocol: Option<String>,
    pub local_base_url: Option<String>,
    pub credential_placements: Vec<String>,
    pub credential_status: BindingCredentialStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProxySetupInfo {
    pub agent_module_id: String,
    pub proxy_running: bool,
    pub proxy_origin: String,
    pub routes: Vec<AgentProxyRouteSetup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderInput {
    pub id: String,
    pub name: String,
    pub billing_kind: BillingKind,
    pub product_group_id: String,
    pub token_sources: Vec<TokenSource>,
    #[serde(default)]
    pub session_source_bindings: Option<Vec<String>>,
    pub quota_source: Option<String>,
    pub quota_interval_seconds: Option<u64>,
    pub route_app_type: Option<String>,
    pub route_config: Option<Value>,
    pub quota_config: Option<Value>,
    pub enabled: bool,
}

impl UsageProviderInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("provider id must not be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err("provider name must not be empty".to_string());
        }
        if self.product_group_id.trim().is_empty() {
            return Err("product group id must not be empty".to_string());
        }
        if matches!(self.quota_interval_seconds, Some(1..=59)) {
            return Err("quota interval must be 0 or at least 60 seconds".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderStored {
    pub id: String,
    pub name: String,
    pub billing_kind: BillingKind,
    pub product_group_id: String,
    pub token_sources: Vec<TokenSource>,
    pub quota_source: Option<String>,
    pub quota_interval_seconds: Option<u64>,
    pub route_app_type: Option<String>,
    pub route_config: Option<Value>,
    pub quota_config: Option<Value>,
    pub enabled: bool,
    pub needs_review: bool,
    pub legacy_app_type: Option<String>,
    pub legacy_provider_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub system_preset_key: Option<String>,
    pub daily_budget_usd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderView {
    pub id: String,
    pub name: String,
    pub billing_kind: BillingKind,
    pub product_group_id: String,
    pub token_sources: Vec<TokenSource>,
    pub session_source_bindings: Vec<String>,
    pub bindings: Vec<AgentProviderBindingView>,
    pub quota_source: Option<String>,
    pub quota_interval_seconds: Option<u64>,
    pub route_app_type: Option<String>,
    pub enabled: bool,
    pub needs_review: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub route_base_url: Option<String>,
    pub has_route_credentials: bool,
    pub system_preset_key: Option<String>,
    pub system_auth_kind: Option<SystemProviderAuthKind>,
    pub canonical_endpoint: Option<String>,
    pub compatible_agent_module_ids: Vec<String>,
    pub upstream_credential_status: BindingCredentialStatus,
    pub upstream_credential_version: u64,
    pub can_clear_upstream_credential: bool,
    pub last_connection_test_at: Option<i64>,
    pub last_connection_test_status: Option<String>,
    pub daily_budget_usd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBinding {
    pub protocol: String,
    pub provider_id: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSourceBinding {
    pub source: String,
    pub provider_id: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub event_id: String,
    pub source: TokenSource,
    pub provider_id: String,
    /// Immutable Agent ownership captured at ingestion time. Legacy events
    /// that cannot be attributed safely keep this value null.
    pub agent_module_id: Option<String>,
    pub product_group_id: String,
    pub occurred_at: i64,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub upstream_correlation_id: Option<String>,
    pub input_cost_usd: Option<String>,
    pub output_cost_usd: Option<String>,
    pub cache_read_cost_usd: Option<String>,
    pub cache_creation_cost_usd: Option<String>,
    pub total_cost_usd: Option<String>,
    pub cost_source: CostSource,
    pub legacy_request_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventLink {
    pub canonical_event_id: String,
    pub duplicate_event_id: String,
    pub link_kind: String,
    pub link_value: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub snapshot_id: String,
    pub provider_id: String,
    pub fetched_at: i64,
    pub five_hour_utilization_percent: Option<String>,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_utilization_percent: Option<String>,
    pub seven_day_resets_at: Option<String>,
    pub manual_resets_remaining: Option<i64>,
    pub raw_payload: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaFetchState {
    pub provider_id: String,
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventPage {
    pub items: Vec<UsageEvent>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedUsageGroup {
    pub provider_id: String,
    pub source: TokenSource,
    pub event_count: u64,
    pub first_occurred_at: i64,
    pub last_occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedAgentUsageSummary {
    pub agent_module_id: String,
    pub event_count: u64,
    pub first_occurred_at: i64,
    pub last_occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidUsageLinkSummary {
    pub reason: String,
    pub link_count: u64,
    pub first_created_at: i64,
    pub last_created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedUsageDiagnostics {
    pub unassigned_event_count: u64,
    pub unassigned_groups: Vec<UnassignedUsageGroup>,
    pub archived_agent_history: Vec<ArchivedAgentUsageSummary>,
    pub invalid_link_summaries: Vec<InvalidUsageLinkSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostSourceCounts {
    pub upstream: u64,
    pub estimated: u64,
    pub unavailable: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaStatusView {
    pub snapshot_id: String,
    pub fetched_at: i64,
    pub five_hour_utilization_percent: Option<String>,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_utilization_percent: Option<String>,
    pub seven_day_resets_at: Option<String>,
    pub manual_resets_remaining: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageView {
    pub provider: UsageProviderView,
    pub shared_account: bool,
    pub event_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: Option<String>,
    pub cost_source_counts: CostSourceCounts,
    pub quota: Option<QuotaStatusView>,
    pub quota_fetch_state: Option<QuotaFetchState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductUsageView {
    pub product_group_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: Option<String>,
    pub cost_source_counts: CostSourceCounts,
    pub token_sources: Vec<TokenSource>,
    pub subscription_providers: Vec<ProviderUsageView>,
    pub metered_providers: Vec<ProviderUsageView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDashboardView {
    pub agent_module_id: String,
    pub start_at: i64,
    pub end_at: i64,
    pub product_groups: Vec<ProductUsageView>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_enum_values_deserialize_and_other_values_fail() {
        for (raw, expected) in [
            ("subscription", BillingKind::Subscription),
            ("metered", BillingKind::Metered),
        ] {
            assert_eq!(
                serde_json::from_value::<BillingKind>(json!(raw)).expect("billing kind"),
                expected
            );
        }

        for (raw, expected) in [
            ("proxy", TokenSource::Proxy),
            ("session_log", TokenSource::SessionLog),
        ] {
            assert_eq!(
                serde_json::from_value::<TokenSource>(json!(raw)).expect("token source"),
                expected
            );
        }

        for (raw, expected) in [
            ("upstream", CostSource::Upstream),
            ("estimated", CostSource::Estimated),
            ("unavailable", CostSource::Unavailable),
        ] {
            assert_eq!(
                serde_json::from_value::<CostSource>(json!(raw)).expect("cost source"),
                expected
            );
        }

        for invalid in ["Subscription", "sessionLog", "unknown", ""] {
            assert!(serde_json::from_value::<BillingKind>(json!(invalid)).is_err());
            assert!(serde_json::from_value::<TokenSource>(json!(invalid)).is_err());
            assert!(serde_json::from_value::<CostSource>(json!(invalid)).is_err());
        }
    }

    fn valid_provider_input() -> UsageProviderInput {
        UsageProviderInput {
            id: "claude:metered".to_string(),
            name: "Metered".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: Some(60),
            route_app_type: Some("claude".to_string()),
            route_config: Some(json!({"baseUrl": "https://example.com"})),
            quota_config: None,
            enabled: true,
        }
    }

    #[test]
    fn provider_input_rejects_empty_identifiers_and_short_quota_intervals() {
        let mut input = valid_provider_input();
        assert!(input.validate().is_ok());

        for invalid_interval in [1, 59] {
            input.quota_interval_seconds = Some(invalid_interval);
            assert!(input.validate().is_err());
        }

        input = valid_provider_input();
        input.id = "  ".to_string();
        assert!(input.validate().is_err());

        input = valid_provider_input();
        input.name.clear();
        assert!(input.validate().is_err());

        input = valid_provider_input();
        input.product_group_id.clear();
        assert!(input.validate().is_err());
    }

    #[test]
    fn provider_input_uses_camel_case_json_fields() {
        let value = serde_json::to_value(valid_provider_input()).expect("serialize provider input");
        assert_eq!(value["billingKind"], json!("metered"));
        assert_eq!(value["productGroupId"], json!("claude"));
        assert_eq!(value["tokenSources"], json!(["proxy"]));
        assert_eq!(value["quotaIntervalSeconds"], json!(60));
        assert!(value.get("billing_kind").is_none());
    }

    #[test]
    fn agent_module_contract_is_strict_and_uses_camel_case() {
        let input: AgentModuleInput = serde_json::from_value(json!({
            "id": null,
            "name": "Gemini",
            "sortOrder": 4,
            "visible": true
        }))
        .expect("deserialize module input");
        assert_eq!(input.sort_order, 4);

        for legacy in [
            json!({
                "id": null,
                "name": "Legacy",
                "kind": "subscription",
                "sortOrder": 4,
                "visible": true
            }),
            json!({
                "id": "api",
                "name": "API",
                "sortOrder": 4,
                "visible": true
            }),
        ] {
            let error = serde_json::from_value::<AgentModuleInput>(legacy).unwrap_err();
            assert!(error.to_string().contains("invalid_agent_module"));
        }

        let view = AgentModuleView {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            sort_order: 3,
            visible: true,
            is_fixed: true,
            archived_at: None,
            provider_count: 2,
        };
        let value = serde_json::to_value(view).expect("serialize module view");
        assert_eq!(value["sortOrder"], json!(3));
        assert_eq!(value["isFixed"], json!(true));
        assert_eq!(value["archivedAt"], Value::Null);
        assert_eq!(value["providerCount"], json!(2));
        assert!(value.get("kind").is_none());
        assert!(value.get("isSystem").is_none());
    }

    #[test]
    fn provider_input_has_no_legacy_dashboard_membership() {
        let value = serde_json::to_value(valid_provider_input()).unwrap();
        assert!(value.get("dashboardModuleId").is_none());
    }
}
