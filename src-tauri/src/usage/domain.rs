use serde::{Deserialize, Serialize};
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
pub enum CostSource {
    Upstream,
    Estimated,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderInput {
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderView {
    pub id: String,
    pub name: String,
    pub billing_kind: BillingKind,
    pub product_group_id: String,
    pub token_sources: Vec<TokenSource>,
    pub quota_source: Option<String>,
    pub quota_interval_seconds: Option<u64>,
    pub route_app_type: Option<String>,
    pub enabled: bool,
    pub needs_review: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub route_base_url: Option<String>,
    pub has_route_credentials: bool,
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
    pub page: u32,
    pub page_size: u32,
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
    pub subscription_providers: Vec<UsageProviderView>,
    pub metered_providers: Vec<UsageProviderView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDashboardView {
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
}
