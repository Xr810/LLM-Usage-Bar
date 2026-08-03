use llm_usage_bar_lib::{
    get_provider_usage_dashboard_test_hook, get_provider_usage_events_test_hook,
    get_route_bindings_test_hook, get_unassigned_usage_diagnostics_test_hook,
    get_usage_dashboard_test_hook, get_usage_events_test_hook,
    list_agent_provider_bindings_test_hook, list_usage_providers_test_hook,
    refresh_provider_quota_test_hook, save_agent_provider_binding_test_hook,
    save_usage_provider_test_hook, set_route_binding_test_hook,
    set_usage_provider_enabled_test_hook, sync_provider_session_usage_test_hook,
    AgentProviderBindingInput, BillingKind, CostSource, TokenSource, UsageEvent,
    UsageProviderInput,
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
        session_source_bindings: None,
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
        session_source_bindings: Some(vec!["claude".to_string()]),
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
        agent_module_id: Some("claude-code".to_string()),
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
        pricing_origin: None,
        legacy_request_id: None,
        created_at: 50,
    }
}

#[tokio::test]
// This guard serializes process-wide HOME mutations with synchronous integration tests.
#[allow(clippy::await_holding_lock)]
async fn nine_command_adapters_validate_and_never_serialize_provider_secrets() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();
    let state = create_test_state().expect("create test state");

    let saved = save_usage_provider_test_hook(&state, metered())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&saved)
        .unwrap()
        .contains("route-secret"));
    let providers = list_usage_providers_test_hook(&state).await.unwrap();
    let system_provider_count = providers
        .iter()
        .filter(|provider| provider.system_preset_key.is_some())
        .count();
    assert_eq!(providers.len(), system_provider_count + 1);
    assert_eq!(
        providers
            .iter()
            .filter(|provider| provider.system_preset_key.is_none())
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        vec!["metered"]
    );
    assert!(providers.iter().any(|provider| provider.id == "metered"));
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

    let empty_dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "claude-code")
        .await
        .unwrap();
    assert_eq!(empty_dashboard.product_groups.len(), 1);
    assert_eq!(
        empty_dashboard.product_groups[0].subscription_providers[0]
            .provider
            .id,
        "system-claude-subscription"
    );
    assert_eq!(
        empty_dashboard.product_groups[0].subscription_providers[0].event_count,
        0
    );

    state.db.insert_usage_event(&event()).unwrap();
    let provider_dashboard = get_provider_usage_dashboard_test_hook(&state, 0, 100).unwrap();
    let provider_row = provider_dashboard
        .providers
        .iter()
        .find(|row| row.provider.id == "metered")
        .unwrap();
    assert_eq!(provider_row.input_tokens, 10);
    assert_eq!(provider_row.total_cost_usd.as_deref(), Some("0.3"));
    let provider_events =
        get_provider_usage_events_test_hook(&state, "metered", 0, 100, 1, 10).unwrap();
    assert_eq!(provider_events.total, 1);
    assert_eq!(provider_events.items[0].event_id, "event");
    let dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "claude-code")
        .await
        .unwrap();
    assert_eq!(dashboard.product_groups.len(), 2);
    let metered_product = dashboard
        .product_groups
        .iter()
        .find(|product| product.product_group_id == "product")
        .unwrap();
    assert_eq!(metered_product.input_tokens, 10);
    assert_eq!(metered_product.total_cost_usd.as_deref(), Some("0.3"));
    assert!(!serde_json::to_string(&dashboard)
        .unwrap()
        .contains("route-secret"));

    for (page, page_size) in [(0, 10), (1, 0), (1, 201)] {
        assert!(get_usage_events_test_hook(
            &state,
            "claude-code",
            Some("metered"),
            0,
            100,
            page,
            page_size
        )
        .is_err());
    }
    assert!(
        get_usage_events_test_hook(&state, "claude-code", Some("missing"), 0, 100, 1, 10).is_err()
    );
    let events =
        get_usage_events_test_hook(&state, "claude-code", Some("metered"), 0, 100, 1, 10).unwrap();
    assert_eq!(events.total, 1);
    assert_eq!(events.items[0].event_id, "event");
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("route-secret"));
    assert_eq!(
        get_usage_events_test_hook(&state, "claude-code", Some("metered"), 51, 100, 1, 10,)
            .unwrap()
            .total,
        0
    );
    assert!(
        get_usage_dashboard_test_hook(&state, 100, 100, "claude-code")
            .await
            .is_err()
    );

    let mut unassigned = event();
    unassigned.event_id = "unassigned".to_string();
    unassigned.agent_module_id = None;
    state.db.insert_usage_event(&unassigned).unwrap();
    let diagnostics = get_unassigned_usage_diagnostics_test_hook(&state).unwrap();
    assert_eq!(diagnostics.unassigned_event_count, 1);
    assert_eq!(diagnostics.unassigned_groups[0].provider_id, "metered");

    assert!(refresh_provider_quota_test_hook(&state, "missing")
        .await
        .is_err());
    let subscription_saved = save_usage_provider_test_hook(&state, subscription())
        .await
        .unwrap();
    assert_eq!(subscription_saved.session_source_bindings, vec!["claude"]);
    assert!(!serde_json::to_string(&subscription_saved)
        .unwrap()
        .contains("quota-secret"));
    assert!(refresh_provider_quota_test_hook(&state, "subscription")
        .await
        .is_err());
    let subscription_binding = save_agent_provider_binding_test_hook(
        &state,
        AgentProviderBindingInput {
            id: None,
            agent_module_id: "claude-code".to_string(),
            provider_id: "subscription".to_string(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        list_agent_provider_bindings_test_hook(&state, Some("claude-code"))
            .await
            .unwrap()
            .iter()
            .filter(|binding| binding.id == subscription_binding.id)
            .count(),
        1
    );
    let failed_quota_dashboard = get_usage_dashboard_test_hook(&state, 0, 100, "claude-code")
        .await
        .unwrap();
    let subscription = failed_quota_dashboard
        .product_groups
        .iter()
        .flat_map(|product| product.subscription_providers.iter())
        .find(|usage| usage.provider.id == "subscription")
        .unwrap();
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

#[test]
fn all_five_agent_compatibility_commands_are_registered_once() {
    let source = include_str!("../src/lib.rs");
    for command in [
        "list_dashboard_modules",
        "save_dashboard_module",
        "reorder_dashboard_modules",
        "set_dashboard_module_visibility",
        "delete_dashboard_module",
    ] {
        let registration = format!("commands::{command},");
        assert_eq!(
            source.matches(&registration).count(),
            1,
            "{command} must be registered exactly once"
        );
    }
}

#[test]
fn all_task_six_agent_commands_and_legacy_routes_are_registered_once() {
    let source = include_str!("../src/lib.rs");
    for command in [
        "list_dashboard_modules",
        "save_dashboard_module",
        "reorder_dashboard_modules",
        "set_dashboard_module_visibility",
        "delete_dashboard_module",
        "list_agent_provider_bindings",
        "save_agent_provider_binding",
        "delete_agent_provider_binding",
        "set_agent_provider_binding_api_key",
        "replace_agent_provider_binding_api_key",
        "clear_agent_provider_binding_api_key",
        "get_agent_proxy_setup_info",
        "get_unassigned_usage_diagnostics",
        "get_usage_dashboard",
        "get_usage_events",
    ] {
        let registration = format!("commands::{command},");
        assert_eq!(
            source.matches(&registration).count(),
            1,
            "{command} must be registered exactly once"
        );
    }
    for legacy in ["get_route_bindings", "set_route_binding"] {
        let registration = format!("commands::{legacy},");
        assert_eq!(source.matches(&registration).count(), 1);
    }
}
