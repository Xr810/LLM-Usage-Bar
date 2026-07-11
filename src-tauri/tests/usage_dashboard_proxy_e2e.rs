use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use cc_switch_lib::{
    usage::dashboard::UsageDashboardService, BillingKind, CostSource, Database, Provider,
    ProxyService, TokenSource, UsageEventPage, UsageProviderInput,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::oneshot;

async fn mock_messages(
    State(hits): State<Arc<AtomicUsize>>,
    Json(request): Json<Value>,
) -> Response {
    let request_index = hits.fetch_add(1, Ordering::SeqCst);
    if request["model"] == "rectifier-retry-probe" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "type": "invalid_request_error",
                    "message": "Invalid signature in thinking block"
                }
            })),
        )
            .into_response();
    }
    let mut usage = json!({
        "input_tokens": if request_index == 0 { 10 } else { 20 },
        "output_tokens": if request_index == 0 { 2 } else { 4 },
        "cache_read_input_tokens": if request_index == 0 { 3 } else { 0 },
        "cache_creation_input_tokens": if request_index == 0 { 4 } else { 0 }
    });
    if request_index == 0 {
        usage["total_cost"] = json!("0.42");
        usage["cost_details"] = json!({
            "input_cost": "0.10",
            "output_cost": "0.20",
            "cache_read_cost": "0.03",
            "cache_creation_cost": "0.09"
        });
    }

    Json(json!({
        "id": format!("msg-e2e-{request_index}"),
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5-20250929",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": usage
    }))
    .into_response()
}

async fn start_mock_upstream(
    hits: Arc<AtomicUsize>,
) -> (std::net::SocketAddr, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let address = listener.local_addr().expect("mock upstream address");
    let app = Router::new()
        .route("/v1/messages", post(mock_messages))
        .with_state(hits);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve mock upstream");
    });
    (address, shutdown_tx)
}

fn metered_provider(upstream: std::net::SocketAddr) -> UsageProviderInput {
    UsageProviderInput {
        id: "metered-e2e".to_string(),
        name: "Metered E2E".to_string(),
        billing_kind: BillingKind::Metered,
        product_group_id: "claude-e2e".to_string(),
        token_sources: vec![TokenSource::Proxy],
        session_source_bindings: None,
        quota_source: None,
        quota_interval_seconds: None,
        route_app_type: Some("claude".to_string()),
        route_config: Some(json!({
            "baseUrl": format!("http://{upstream}"),
            "apiKey": "fixture-only-key"
        })),
        quota_config: None,
        enabled: true,
    }
}

fn legacy_provider(id: &str, upstream: std::net::SocketAddr) -> Provider {
    Provider::with_id(
        id.to_string(),
        format!("Legacy {id}"),
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{upstream}"),
                "ANTHROPIC_AUTH_TOKEN": "legacy-fixture-key"
            }
        }),
        None,
    )
}

async fn start_proxy(db: Arc<Database>) -> (ProxyService, u16) {
    let proxy = ProxyService::new(db);
    let mut config = proxy.get_config().await.expect("read proxy config");
    config.listen_address = "127.0.0.1".to_string();
    config.listen_port = 0;
    config.enable_logging = true;
    proxy
        .update_config(&config)
        .await
        .expect("configure proxy on port zero");
    let info = proxy.start().await.expect("start proxy on port zero");
    (proxy, info.port)
}

async fn send_message(client: &reqwest::Client, port: u16) -> reqwest::Response {
    client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .json(&json!({
            "model": "claude-sonnet-4-5-20250929",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("send request through proxy")
}

async fn send_rectifier_retry_probe(client: &reqwest::Client, port: u16) -> reqwest::Response {
    client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .json(&json!({
            "model": "rectifier-retry-probe",
            "max_tokens": 16,
            "stream": false,
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "thinking",
                        "thinking": "private",
                        "signature": "invalid"
                    }]
                },
                {"role": "user", "content": "continue"}
            ]
        }))
        .send()
        .await
        .expect("send rectifier retry probe through proxy")
}

async fn wait_for_events(
    db: &Database,
    start_at: i64,
    end_at: i64,
    expected: u64,
) -> UsageEventPage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = db
                .list_usage_events("metered-e2e", start_at, end_at, 1, 20)
                .expect("query usage events");
            if page.total == expected {
                return page;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("usage ingestion completed")
}

#[tokio::test]
async fn real_proxy_requests_feed_dashboard_and_route_errors_stay_local() {
    let hits = Arc::new(AtomicUsize::new(0));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    db.set_route_binding("claude", "metered-e2e")
        .expect("bind Claude route");

    let (proxy, proxy_port) = start_proxy(db.clone()).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    let explicit = send_message(&client, proxy_port).await;
    assert_eq!(explicit.status(), reqwest::StatusCode::OK);
    let explicit_body: Value = explicit.json().await.expect("explicit upstream response");
    assert_eq!(explicit_body["id"], "msg-e2e-0");

    let estimated = send_message(&client, proxy_port).await;
    assert_eq!(estimated.status(), reqwest::StatusCode::OK);
    let estimated_body: Value = estimated.json().await.expect("estimated upstream response");
    assert_eq!(estimated_body["id"], "msg-e2e-1");

    let hits_before_probe = hits.load(Ordering::SeqCst);
    let retry_probe = send_rectifier_retry_probe(&client, proxy_port).await;
    assert_eq!(retry_probe.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        hits_before_probe + 1,
        "the static v13 route must make exactly one upstream attempt"
    );

    // The ingestion worker may use the full five-second polling allowance; keep
    // the half-open query range wider than that allowance so late completion
    // cannot fall outside `occurred_at < end_at`.
    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    assert_eq!(events.total, 2);
    assert_eq!(events.items.len(), 2);
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    let upstream_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-e2e-0"))
        .expect("explicit-cost event");
    assert_eq!(upstream_event.provider_id, "metered-e2e");
    assert!(upstream_event.occurred_at >= start_at && upstream_event.occurred_at < end_at);
    assert_eq!(upstream_event.input_tokens, 10);
    assert_eq!(upstream_event.output_tokens, 2);
    assert_eq!(upstream_event.cache_read_tokens, 3);
    assert_eq!(upstream_event.cache_creation_tokens, 4);
    assert_eq!(upstream_event.input_cost_usd.as_deref(), Some("0.10"));
    assert_eq!(upstream_event.output_cost_usd.as_deref(), Some("0.20"));
    assert_eq!(upstream_event.cache_read_cost_usd.as_deref(), Some("0.03"));
    assert_eq!(
        upstream_event.cache_creation_cost_usd.as_deref(),
        Some("0.09")
    );
    assert_eq!(upstream_event.total_cost_usd.as_deref(), Some("0.42"));
    assert_eq!(upstream_event.cost_source, CostSource::Upstream);

    let estimated_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-e2e-1"))
        .expect("estimated-cost event");
    assert_eq!(estimated_event.input_tokens, 20);
    assert_eq!(estimated_event.output_tokens, 4);
    assert_eq!(estimated_event.cost_source, CostSource::Estimated);
    assert_eq!(estimated_event.total_cost_usd.as_deref(), Some("0.00012"));

    let dashboard = UsageDashboardService::new(&db)
        .get_dashboard(start_at, end_at, Some("claude-e2e"))
        .expect("query usage dashboard");
    assert_eq!(dashboard.product_groups.len(), 1);
    let product = &dashboard.product_groups[0];
    assert_eq!(product.input_tokens, 30);
    assert_eq!(product.output_tokens, 6);
    assert_eq!(product.cache_read_tokens, 3);
    assert_eq!(product.cache_creation_tokens, 4);
    assert_eq!(product.total_cost_usd.as_deref(), Some("0.42012"));
    assert_eq!(product.cost_source_counts.upstream, 1);
    assert_eq!(product.cost_source_counts.estimated, 1);
    assert_eq!(product.cost_source_counts.unavailable, 0);
    assert_eq!(product.token_sources, vec![TokenSource::Proxy]);
    assert!(product.subscription_providers.is_empty());
    assert_eq!(product.metered_providers.len(), 1);
    let provider_summary = &product.metered_providers[0];
    assert_eq!(provider_summary.provider.id, "metered-e2e");
    assert_eq!(provider_summary.provider.name, "Metered E2E");
    assert_eq!(provider_summary.provider.billing_kind, BillingKind::Metered);
    assert_eq!(provider_summary.provider.product_group_id, "claude-e2e");
    assert_eq!(
        provider_summary.provider.token_sources,
        vec![TokenSource::Proxy]
    );
    assert!(provider_summary.provider.quota_source.is_none());
    assert!(provider_summary.provider.quota_interval_seconds.is_none());
    assert_eq!(
        provider_summary.provider.route_app_type.as_deref(),
        Some("claude")
    );
    assert!(provider_summary.provider.enabled);
    assert!(!provider_summary.provider.needs_review);
    assert_eq!(
        provider_summary.provider.route_base_url.as_deref(),
        Some(format!("http://{upstream}").as_str())
    );
    assert!(provider_summary.provider.has_route_credentials);
    assert!(provider_summary.provider.created_at > 0);
    assert!(provider_summary.provider.updated_at >= provider_summary.provider.created_at);
    assert_eq!(provider_summary.event_count, 2);
    assert_eq!(provider_summary.input_tokens, 30);
    assert_eq!(provider_summary.output_tokens, 6);
    assert_eq!(provider_summary.cache_read_tokens, 3);
    assert_eq!(provider_summary.cache_creation_tokens, 4);
    assert_eq!(provider_summary.total_cost_usd.as_deref(), Some("0.42012"));
    assert_eq!(provider_summary.cost_source_counts.upstream, 1);
    assert_eq!(provider_summary.cost_source_counts.estimated, 1);
    assert_eq!(provider_summary.cost_source_counts.unavailable, 0);
    assert!(provider_summary.quota.is_none());
    assert!(provider_summary.quota_fetch_state.is_none());

    let unbound_db = Arc::new(Database::memory().expect("unbound in-memory database"));
    let legacy_current = legacy_provider("legacy-current", upstream);
    let legacy_failover = legacy_provider("legacy-failover", upstream);
    unbound_db
        .save_provider("claude", &legacy_current)
        .expect("save legacy current provider");
    unbound_db
        .save_provider("claude", &legacy_failover)
        .expect("save legacy failover provider");
    unbound_db
        .set_current_provider("claude", &legacy_current.id)
        .expect("select legacy current provider");
    unbound_db
        .add_to_failover_queue("claude", &legacy_failover.id)
        .expect("queue legacy failover provider");
    let (unbound_proxy, unbound_port) = start_proxy(unbound_db).await;
    let hits_before_503 = hits.load(Ordering::SeqCst);
    let route_less = send_message(&client, unbound_port).await;
    assert_eq!(
        route_less.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
    let route_error: Value = route_less.json().await.expect("local route error body");
    assert_eq!(route_error["error"]["type"], "proxy_error");
    assert!(route_error["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("claude")));
    assert_eq!(hits.load(Ordering::SeqCst), hits_before_503);

    unbound_proxy.stop().await.expect("stop unbound proxy");
    proxy.stop().await.expect("stop bound proxy");
    let _ = upstream_shutdown.send(());
}
