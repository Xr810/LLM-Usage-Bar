use axum::{extract::State, routing::post, Json, Router};
use cc_switch_lib::{
    usage::dashboard::UsageDashboardService, BillingKind, CostSource, Database, ProxyConfig,
    ProxyServer, TokenSource, UsageEventPage, UsageProviderInput,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::oneshot;

async fn mock_messages(State(hits): State<Arc<AtomicUsize>>) -> Json<Value> {
    let request_index = hits.fetch_add(1, Ordering::SeqCst);
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

fn proxy_config() -> ProxyConfig {
    ProxyConfig {
        listen_address: "127.0.0.1".to_string(),
        listen_port: 0,
        enable_logging: true,
        ..ProxyConfig::default()
    }
}

fn metered_provider(upstream: std::net::SocketAddr) -> UsageProviderInput {
    UsageProviderInput {
        id: "metered-e2e".to_string(),
        name: "Metered E2E".to_string(),
        billing_kind: BillingKind::Metered,
        product_group_id: "claude-e2e".to_string(),
        token_sources: vec![TokenSource::Proxy],
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

    let proxy = ProxyServer::new(proxy_config(), db.clone(), None);
    let proxy_info = proxy.start().await.expect("start proxy on port zero");
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    let explicit = send_message(&client, proxy_info.port).await;
    assert_eq!(explicit.status(), reqwest::StatusCode::OK);
    let explicit_body: Value = explicit.json().await.expect("explicit upstream response");
    assert_eq!(explicit_body["id"], "msg-e2e-0");

    let estimated = send_message(&client, proxy_info.port).await;
    assert_eq!(estimated.status(), reqwest::StatusCode::OK);
    let estimated_body: Value = estimated.json().await.expect("estimated upstream response");
    assert_eq!(estimated_body["id"], "msg-e2e-1");

    let end_at = chrono::Utc::now().timestamp() + 2;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);

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
    assert_eq!(product.cost_source_counts.upstream, 1);
    assert_eq!(product.cost_source_counts.estimated, 1);
    assert_eq!(product.metered_providers.len(), 1);
    assert_eq!(product.metered_providers[0].provider.id, "metered-e2e");

    let unbound_db = Arc::new(Database::memory().expect("unbound in-memory database"));
    let unbound_proxy = ProxyServer::new(proxy_config(), unbound_db, None);
    let unbound_info = unbound_proxy
        .start()
        .await
        .expect("start unbound proxy on port zero");
    let hits_before_503 = hits.load(Ordering::SeqCst);
    let route_less = send_message(&client, unbound_info.port).await;
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
