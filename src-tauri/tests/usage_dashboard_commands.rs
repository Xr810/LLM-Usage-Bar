use cc_switch_lib::{
    get_route_bindings_test_hook, get_usage_dashboard_test_hook, get_usage_events_test_hook,
    list_usage_providers_test_hook, refresh_provider_quota_test_hook,
    save_usage_provider_test_hook, set_route_binding_test_hook,
    set_usage_provider_enabled_test_hook, sync_provider_session_usage_test_hook, BillingKind,
    CostSource, TokenSource, UsageEvent, UsageProviderInput,
};
use serde_json::json;

#[path = "support.rs"]
mod support;
use support::{create_test_state, ensure_test_home, reset_test_fs, test_mutex};

fn metered() -> UsageProviderInput {
    UsageProviderInput {
        id: "metered".to_string(),
        name: "Metered".to_string(),
        billing_kind: BillingKind::Metered,
        product_group_id: "product".to_string(),
        token_sources: vec![TokenSource::Proxy],
        quota_source: None,
        quota_interval_seconds: None,
        route_app_type: Some("claude".to_string()),
        route_config: Some(json!({
            "baseUrl": "https://example.com",
            "apiKey": "route-secret"
        })),
        quota_config: None,
        enabled: true,
    }
}

fn subscription() -> UsageProviderInput {
    UsageProviderInput {
        id: "subscription".to_string(),
        name: "Subscription".to_string(),
        billing_kind: BillingKind::Subscription,
        product_group_id: "product".to_string(),
        token_sources: vec![TokenSource::SessionLog],
        quota_source: Some("unsupported-test-source".to_string()),
        quota_interval_seconds: Some(300),
        route_app_type: None,
        route_config: None,
        quota_config: None,
        enabled: true,
    }
}

fn event() -> UsageEvent {
    UsageEvent {
        event_id: "event".to_string(),
        source: TokenSource::Proxy,
        provider_id: "metered".to_string(),
        product_group_id: "product".to_string(),
        occurred_at: 50,
        model: "model".to_string(),
        input_tokens: 10,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_creation_tokens: 4,
        request_id: Some("request".to_string()),
        session_id: None,
        upstream_correlation_id: None,
        input_cost_usd: Some("0.1".to_string()),
        output_cost_usd: Some("0.2".to_string()),
        cache_read_cost_usd: None,
        cache_creation_cost_usd: None,
        total_cost_usd: Some("0.3".to_string()),
        cost_source: CostSource::Upstream,
        legacy_request_id: None,
        created_at: 50,
    }
}

#[tokio::test]
async fn nine_command_adapters_validate_and_never_serialize_provider_secrets() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();
    let state = create_test_state().expect("create test state");

    let saved = save_usage_provider_test_hook(&state, metered()).unwrap();
    assert!(!serde_json::to_string(&saved)
        .unwrap()
        .contains("route-secret"));
    let providers = list_usage_providers_test_hook(&state).unwrap();
    assert_eq!(providers.len(), 1);
    assert!(!serde_json::to_string(&providers)
        .unwrap()
        .contains("route-secret"));

    set_usage_provider_enabled_test_hook(&state, "metered", false).unwrap();
    assert!(set_route_binding_test_hook(&state, "claude", "metered").is_err());
    set_usage_provider_enabled_test_hook(&state, "metered", true).unwrap();
    let binding = set_route_binding_test_hook(&state, "claude", "metered").unwrap();
    assert!(!serde_json::to_string(&binding)
        .unwrap()
        .contains("route-secret"));
    let bindings = get_route_bindings_test_hook(&state).unwrap();
    assert_eq!(bindings.len(), 1);
    assert!(!serde_json::to_string(&bindings)
        .unwrap()
        .contains("route-secret"));

    let empty_dashboard = get_usage_dashboard_test_hook(&state, 0, 100, None).unwrap();
    assert_eq!(empty_dashboard.product_groups[0].input_tokens, 0);

    state.db.insert_usage_event(&event()).unwrap();
    let dashboard = get_usage_dashboard_test_hook(&state, 0, 100, None).unwrap();
    assert_eq!(dashboard.product_groups.len(), 1);
    assert_eq!(dashboard.product_groups[0].input_tokens, 10);
    assert_eq!(
        dashboard.product_groups[0].total_cost_usd.as_deref(),
        Some("0.3")
    );
    assert!(!serde_json::to_string(&dashboard)
        .unwrap()
        .contains("route-secret"));

    for (page, page_size) in [(0, 10), (1, 0), (1, 201)] {
        assert!(get_usage_events_test_hook(&state, "metered", 0, 100, page, page_size).is_err());
    }
    assert!(get_usage_events_test_hook(&state, "missing", 0, 100, 1, 10).is_err());
    let events = get_usage_events_test_hook(&state, "metered", 0, 100, 1, 10).unwrap();
    assert_eq!(events.total, 1);
    assert_eq!(events.items[0].event_id, "event");
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("route-secret"));
    assert_eq!(
        get_usage_events_test_hook(&state, "metered", 51, 100, 1, 10)
            .unwrap()
            .total,
        0
    );
    assert!(get_usage_dashboard_test_hook(&state, 100, 100, None).is_err());

    assert!(refresh_provider_quota_test_hook(&state, "missing")
        .await
        .is_err());
    save_usage_provider_test_hook(&state, subscription()).unwrap();
    assert!(refresh_provider_quota_test_hook(&state, "subscription")
        .await
        .is_err());
    let failed_quota_dashboard = get_usage_dashboard_test_hook(&state, 0, 100, None).unwrap();
    let subscription = &failed_quota_dashboard.product_groups[0].subscription_providers[0];
    assert!(subscription.quota.is_none());
    assert!(subscription
        .quota_fetch_state
        .as_ref()
        .and_then(|state| state.last_error.as_deref())
        .is_some());

    let session = sync_provider_session_usage_test_hook(&state, "metered").unwrap();
    assert!(!session.warnings.is_empty());
    assert!(!serde_json::to_string(&session)
        .unwrap()
        .contains("route-secret"));
}

#[test]
fn all_nine_public_commands_are_registered_once() {
    let source = include_str!("../src/lib.rs");
    for command in [
        "list_usage_providers",
        "save_usage_provider",
        "set_usage_provider_enabled",
        "get_route_bindings",
        "set_route_binding",
        "get_usage_dashboard",
        "get_usage_events",
        "refresh_provider_quota",
        "sync_provider_session_usage",
    ] {
        let registration = format!("commands::{command},");
        assert_eq!(
            source.matches(&registration).count(),
            1,
            "{command} must be registered exactly once"
        );
    }
}
