use axum::{
    body::Body,
    extract::{OriginalUri, State},
    http::{header::CONTENT_TYPE, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, post},
    Json, Router,
};
use llm_usage_bar_lib::{
    credentials::{BindingCredentialService, CredentialStore, CredentialStoreError, SecretString},
    usage::dashboard::UsageDashboardService,
    AgentModuleInput, AgentProviderBindingInput, AgentProviderBindingView, BillingKind, CostSource,
    Database, Provider, ProxyService, TokenSource, UsageEventPage, UsageProviderInput,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Condvar, Mutex, Once,
};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};

#[derive(Default)]
struct MemoryCredentialStore {
    secrets: Mutex<HashMap<String, Vec<u8>>>,
    get_barrier: Mutex<GetBarrierState>,
    get_barrier_cv: Condvar,
}

#[derive(Default)]
struct GetBarrierState {
    target_secret: Option<Vec<u8>>,
    remaining_matches_to_skip: usize,
    captured: bool,
    released: bool,
}

impl CredentialStore for MemoryCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        self.secrets
            .lock()
            .expect("credential store lock")
            .insert(slot.to_string(), secret.to_vec());
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        let value = self
            .secrets
            .lock()
            .expect("credential store lock")
            .get(slot)
            .cloned();
        let mut barrier = self.get_barrier.lock().expect("get barrier lock");
        if value
            .as_deref()
            .zip(barrier.target_secret.as_deref())
            .is_some_and(|(value, target)| value == target)
        {
            if barrier.remaining_matches_to_skip > 0 {
                barrier.remaining_matches_to_skip -= 1;
            } else {
                barrier.captured = true;
                self.get_barrier_cv.notify_all();
                while !barrier.released {
                    barrier = self.get_barrier_cv.wait(barrier).expect("get barrier wait");
                }
            }
        }
        Ok(value)
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        self.secrets
            .lock()
            .expect("credential store lock")
            .remove(slot);
        Ok(())
    }
}

impl MemoryCredentialStore {
    fn remove_secret_value(&self, secret: &str) {
        self.secrets
            .lock()
            .expect("credential store lock")
            .retain(|_, value| value.as_slice() != secret.as_bytes());
    }

    fn arm_get_barrier_after_matches(&self, secret: &str, matches_to_skip: usize) {
        let mut barrier = self.get_barrier.lock().expect("get barrier lock");
        barrier.target_secret = Some(secret.as_bytes().to_vec());
        barrier.remaining_matches_to_skip = matches_to_skip;
        barrier.captured = false;
        barrier.released = false;
    }

    fn get_barrier_captured(&self) -> bool {
        self.get_barrier.lock().expect("get barrier lock").captured
    }

    fn release_get_barrier(&self) {
        let mut barrier = self.get_barrier.lock().expect("get barrier lock");
        barrier.released = true;
        self.get_barrier_cv.notify_all();
    }
}

#[derive(Default)]
struct StreamResponseGate {
    reached: AtomicBool,
    released: AtomicBool,
    reached_notify: Notify,
    release_notify: Notify,
}

impl StreamResponseGate {
    fn mark_reached(&self) {
        self.reached.store(true, Ordering::SeqCst);
        self.reached_notify.notify_waiters();
    }

    async fn wait_until_reached(&self) {
        while !self.reached.load(Ordering::SeqCst) {
            self.reached_notify.notified().await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.release_notify.notify_waiters();
    }

    async fn wait_until_released(&self) {
        while !self.released.load(Ordering::SeqCst) {
            self.release_notify.notified().await;
        }
    }
}

#[derive(Debug, Clone)]
struct CapturedUpstreamRequest {
    headers: Vec<(String, String)>,
    uri: String,
    body: Value,
}

struct CapturingAppLogger;

static CAPTURING_APP_LOGGER: CapturingAppLogger = CapturingAppLogger;
static APP_LOGGER_INIT: Once = Once::new();
static CAPTURED_APP_LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl log::Log for CapturingAppLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        CAPTURED_APP_LOGS
            .lock()
            .expect("captured app logs lock")
            .push(record.args().to_string());
    }

    fn flush(&self) {}
}

fn ensure_app_log_capture() {
    APP_LOGGER_INIT.call_once(|| {
        log::set_logger(&CAPTURING_APP_LOGGER).expect("test logger should initialize once");
        log::set_max_level(log::LevelFilter::Trace);
    });
}

fn assert_app_logs_omit_binding_keys(binding_keys: &[&str]) {
    let logs = CAPTURED_APP_LOGS
        .lock()
        .expect("captured app logs lock")
        .join("\n");
    for binding_key in binding_keys {
        assert!(
            !logs.contains(binding_key),
            "app logs retained a binding key"
        );
    }
}

#[derive(Clone)]
struct MockUpstreamState {
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<CapturedUpstreamRequest>>>,
    stream_response_gate: Option<Arc<StreamResponseGate>>,
}

#[derive(Clone)]
struct RedirectUpstreamState {
    hits: Arc<AtomicUsize>,
    location: String,
}

async fn redirecting_upstream(State(state): State<RedirectUpstreamState>) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let mut response = (
        StatusCode::FOUND,
        Json(json!({"error": "redirect details must remain upstream-only"})),
    )
        .into_response();
    response.headers_mut().insert(
        "location",
        state.location.parse().expect("redirect Location header"),
    );
    response
        .headers_mut()
        .insert("x-upstream-redirect", state.location.parse().unwrap());
    response
}

async fn redirect_target(State(hits): State<Arc<AtomicUsize>>) -> Response {
    hits.fetch_add(1, Ordering::SeqCst);
    Json(json!({"followed": true})).into_response()
}

async fn mock_messages(
    State(state): State<MockUpstreamState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    state
        .requests
        .lock()
        .expect("captured requests lock")
        .push(CapturedUpstreamRequest {
            headers: headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().unwrap_or("<non-utf8>").to_string(),
                    )
                })
                .collect(),
            uri: uri.to_string(),
            body: request.clone(),
        });
    let request_index = state.hits.fetch_add(1, Ordering::SeqCst);
    if request["model"] == "frozen-stream-probe" {
        let gate = state
            .stream_response_gate
            .as_ref()
            .expect("frozen stream probe requires a response gate");
        gate.mark_reached();
        gate.wait_until_released().await;
        let body = format!(
            concat!(
                "event: message_start\n",
                "data: {{\"type\":\"message_start\",\"message\":{{",
                "\"id\":\"msg-stream-frozen-{request_index}\",",
                "\"type\":\"message\",\"role\":\"assistant\",",
                "\"model\":\"claude-sonnet-4-5-20250929\",",
                "\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,",
                "\"usage\":{{\"input_tokens\":11,\"output_tokens\":0,",
                "\"cache_read_input_tokens\":2,\"cache_creation_input_tokens\":3,",
                "\"total_cost\":\"0.17\"}}}}}}\n\n",
                "event: content_block_start\n",
                "data: {{\"type\":\"content_block_start\",\"index\":0,",
                "\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
                "event: content_block_delta\n",
                "data: {{\"type\":\"content_block_delta\",\"index\":0,",
                "\"delta\":{{\"type\":\"text_delta\",\"text\":\"ok\"}}}}\n\n",
                "event: content_block_stop\n",
                "data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
                "event: message_delta\n",
                "data: {{\"type\":\"message_delta\",",
                "\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},",
                "\"usage\":{{\"output_tokens\":5}}}}\n\n",
                "event: message_stop\n",
                "data: {{\"type\":\"message_stop\"}}\n\n"
            ),
            request_index = request_index,
        );
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .body(Body::from(body))
            .expect("build frozen stream response");
    }
    if request["model"] == "credential-success-echo-probe" {
        let echoed_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("missing-key");
        return Json(json!({
            "id": echoed_key,
            "type": "message",
            "role": "assistant",
            "model": echoed_key,
            "content": [{"type": "text", "text": "apparently safe"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 99,
                "output_tokens": 99,
                "total_cost": "99.99"
            }
        }))
        .into_response();
    }
    if request["model"] == "credential-echo-probe" {
        let echoed_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("missing-key");
        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "type": "authentication_error",
                    "message": format!("invalid API key {echoed_key}")
                }
            })),
        )
            .into_response();
        response.headers_mut().insert(
            "x-upstream-credential-echo",
            echoed_key.parse().expect("echo test header"),
        );
        return response;
    }
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
    let serialized_request = request.to_string();
    if uri.path().contains(":streamGenerateContent")
        && serialized_request.contains("split-secret-stream-probe")
    {
        let echoed_key = headers
            .get("x-goog-api-key")
            .and_then(|value| value.to_str().ok())
            .expect("Gemini stream probe protected key");
        let split_at = echoed_key.len() / 2;
        let (first_half, second_half) = echoed_key.split_at(split_at);
        let body = format!(
            concat!(
                "data: {{\"responseId\":\"gemini-split-secret\",",
                "\"modelVersion\":\"gemini-2.5-pro\",",
                "\"candidates\":[{{\"content\":{{\"role\":\"model\",",
                "\"parts\":[{{\"text\":{first_half:?}}}]}}}}]}}\n\n",
                "data: {{\"responseId\":\"gemini-split-secret\",",
                "\"modelVersion\":\"gemini-2.5-pro\",",
                "\"candidates\":[{{\"content\":{{\"role\":\"model\",",
                "\"parts\":[{{\"text\":{second_half:?}}}]}},",
                "\"finishReason\":\"STOP\"}}]}}\n\n"
            ),
            first_half = first_half,
            second_half = second_half,
        );
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .body(Body::from(body))
            .expect("build split-secret Gemini stream response");
    }
    if uri.path().contains(":generateContent")
        && serialized_request.contains("shadow-followup-probe")
    {
        return Json(json!({
            "responseId": format!("gemini-shadow-followup-{request_index}"),
            "modelVersion": "gemini-2.5-pro",
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "safe followup"}]
                },
                "finishReason": "STOP"
            }]
        }))
        .into_response();
    }
    if uri.path().contains(":generateContent") {
        return Json(json!({
            "responseId": format!("gemini-e2e-{request_index}"),
            "modelVersion": "gemini-2.5-pro",
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "ok"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": if request_index == 0 { 7 } else { 11 },
                "candidatesTokenCount": if request_index == 0 { 3 } else { 5 },
                "totalTokenCount": if request_index == 0 { 10 } else { 16 }
            }
        }))
        .into_response();
    }
    if uri.path().ends_with("/responses") {
        return Json(json!({
            "id": format!("resp-e2e-{request_index}"),
            "object": "response",
            "created_at": chrono::Utc::now().timestamp(),
            "status": "completed",
            "model": request["model"],
            "output": [{
                "id": format!("msg-response-e2e-{request_index}"),
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "ok", "annotations": []}]
            }],
            "usage": {
                "input_tokens": 7,
                "output_tokens": 3,
                "total_tokens": 10
            }
        }))
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
    requests: Arc<Mutex<Vec<CapturedUpstreamRequest>>>,
) -> (std::net::SocketAddr, oneshot::Sender<()>) {
    start_mock_upstream_with_stream_gate(hits, requests, None).await
}

async fn start_mock_upstream_with_stream_gate(
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<CapturedUpstreamRequest>>>,
    stream_response_gate: Option<Arc<StreamResponseGate>>,
) -> (std::net::SocketAddr, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let address = listener.local_addr().expect("mock upstream address");
    let app = Router::new()
        .route("/v1/messages", post(mock_messages))
        .fallback(post(mock_messages))
        .with_state(MockUpstreamState {
            hits,
            requests,
            stream_response_gate,
        });
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

async fn start_redirect_target(
    hits: Arc<AtomicUsize>,
) -> (std::net::SocketAddr, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target");
    let address = listener.local_addr().expect("redirect target address");
    let app = Router::new()
        .fallback(any(redirect_target))
        .with_state(hits);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve redirect target");
    });
    (address, shutdown_tx)
}

async fn start_redirecting_upstream(
    hits: Arc<AtomicUsize>,
    location: String,
) -> (std::net::SocketAddr, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirecting upstream");
    let address = listener.local_addr().expect("redirecting upstream address");
    let app = Router::new()
        .fallback(any(redirecting_upstream))
        .with_state(RedirectUpstreamState { hits, location });
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve redirecting upstream");
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
            "apiKey": "old-route-config-secret",
            "nested": {
                "legacySecret": "old-legacy-envelope-secret"
            }
        })),
        quota_config: None,
        enabled: true,
    }
}

fn metered_gemini_native_provider(upstream: std::net::SocketAddr) -> UsageProviderInput {
    let mut provider = metered_provider(upstream);
    provider.route_config = Some(json!({
        "api_format": "gemini_native",
        "env": {
            "ANTHROPIC_BASE_URL": format!("http://{upstream}"),
            "ANTHROPIC_API_KEY": "old-gemini-route-secret"
        }
    }));
    provider
}

fn metered_gemini_provider(upstream: std::net::SocketAddr) -> UsageProviderInput {
    let mut provider = metered_provider(upstream);
    provider.product_group_id = "gemini-e2e".to_string();
    provider.route_app_type = Some("gemini".to_string());
    provider.route_config = Some(json!({
        "baseUrl": format!("http://{upstream}"),
        "apiKey": "old-gemini-route-key"
    }));
    provider
}

fn metered_codex_provider(upstream: std::net::SocketAddr) -> UsageProviderInput {
    let mut provider = metered_provider(upstream);
    provider.product_group_id = "codex-e2e".to_string();
    provider.route_app_type = Some("codex".to_string());
    provider.route_config = Some(json!({
        "base_url": format!("http://{upstream}"),
        "apiFormat": "openai_responses"
    }));
    provider
}

async fn create_enabled_binding(
    db: &Database,
    credentials: &BindingCredentialService,
    agent_module_id: &str,
    key: &str,
) -> AgentProviderBindingView {
    create_enabled_binding_for_provider(db, credentials, agent_module_id, "metered-e2e", key).await
}

async fn create_enabled_binding_for_provider(
    db: &Database,
    credentials: &BindingCredentialService,
    agent_module_id: &str,
    provider_id: &str,
    key: &str,
) -> AgentProviderBindingView {
    ensure_app_log_capture();
    let binding = db
        .save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: agent_module_id.to_string(),
            provider_id: provider_id.to_string(),
            enabled: false,
        })
        .expect("create disabled binding");
    let configured = credentials
        .set_binding_api_key(
            &binding.id,
            binding.credential_version,
            SecretString::new(key.to_string()),
        )
        .await
        .expect("protect binding key");
    db.save_agent_provider_binding(&AgentProviderBindingInput {
        id: Some(binding.id),
        agent_module_id: agent_module_id.to_string(),
        provider_id: provider_id.to_string(),
        enabled: true,
    })
    .unwrap_or_else(|error| {
        panic!(
            "enable configured binding at version {}: {error}",
            configured.credential_version
        )
    })
}

async fn start_proxy(
    db: Arc<Database>,
    credential_store: Arc<dyn CredentialStore>,
) -> (ProxyService, u16) {
    let proxy = ProxyService::new_with_credential_store(db, credential_store);
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

async fn send_message_with_bearer(
    client: &reqwest::Client,
    port: u16,
    bearer: Option<&str>,
) -> reqwest::Response {
    send_message_with_bearer_and_query(client, port, bearer, None).await
}

async fn send_message_with_bearer_and_query(
    client: &reqwest::Client,
    port: u16,
    bearer: Option<&str>,
    query: Option<&str>,
) -> reqwest::Response {
    let url = match query {
        Some(query) => format!("http://127.0.0.1:{port}/v1/messages?{query}"),
        None => format!("http://127.0.0.1:{port}/v1/messages"),
    };
    let mut request = client
        .post(url)
        .json(&json!({
            "model": "claude-sonnet-4-5-20250929",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .header("proxy-authorization", "Bearer inbound-proxy-auth-secret")
        .header("api-key", "inbound-api-key-secret")
        .header("x-auth-token", "inbound-auth-token-secret")
        .header("x-amz-security-token", "inbound-aws-token-secret")
        .header("cookie", "session=inbound-cookie-secret");
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    request.send().await.expect("send request through proxy")
}

#[tokio::test]
async fn claude_setup_base_path_is_stripped_before_forwarding() {
    const BINDING_KEY: &str = "claude-setup-binding-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db, store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let response = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/claude/v1/messages?trace=setup"
        ))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "claude-sonnet-4-5-20250929",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("send request through the Claude setup base path");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri, "/v1/messages?trace=setup");
    assert_eq!(
        captured_header(&requests[0], "x-api-key"),
        Some(BINDING_KEY)
    );

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn codex_setup_base_path_serves_the_model_catalog() {
    let db = Arc::new(Database::memory().expect("in-memory database"));
    let store = Arc::new(MemoryCredentialStore::default());
    let (proxy, proxy_port) = start_proxy(db, store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");

    let response = client
        .get(format!("http://127.0.0.1:{proxy_port}/codex/v1/models"))
        .send()
        .await
        .expect("read models through the Codex setup base path");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("parse model catalog");
    assert!(body.get("models").is_some_and(serde_json::Value::is_array));

    proxy.stop().await.expect("stop proxy");
}

#[tokio::test]
async fn unknown_binding_credential_is_rejected_before_upstream() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    db.set_route_binding("claude", "metered-e2e")
        .expect("install dormant compatibility route");

    let store = Arc::new(MemoryCredentialStore::default());
    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let response = send_message_with_bearer(&client, proxy_port, Some("unknown-binding-key")).await;

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response_body = response.text().await.expect("local rejection body");
    assert_generic_local_rejection(&response_body, "unknown-binding-key");
    let invalid_body_response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth("unknown-binding-key")
        .header("content-type", "application/json")
        .body("not-json-and-must-not-be-parsed")
        .send()
        .await
        .expect("send invalid body with unknown binding key");
    assert_eq!(
        invalid_body_response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "unknown credentials must be rejected before request body parsing"
    );
    let invalid_body_rejection = invalid_body_response
        .text()
        .await
        .expect("pre-body authorization rejection");
    assert_eq!(invalid_body_rejection, response_body);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "an unknown binding key must never reach the static compatibility route"
    );
    let end_at = chrono::Utc::now().timestamp() + 2;
    let events = db
        .list_usage_events("metered-e2e", 0, end_at, 1, 20)
        .expect("query usage events");
    assert_eq!(events.total, 0);
    assert!(captured.lock().expect("captured requests lock").is_empty());
    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn upstream_error_echoes_never_expose_the_protected_binding_key() {
    const BINDING_KEY: &str = "upstream-error-echo-binding-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "credential-echo-probe",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("send upstream error echo probe");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(response
        .headers()
        .get("x-upstream-credential-echo")
        .is_none());
    let response_body = response.text().await.expect("upstream error response body");
    assert!(
        !response_body.contains(BINDING_KEY),
        "HTTP error response leaked the binding key: {response_body}"
    );

    let status_body = client
        .get(format!("http://127.0.0.1:{proxy_port}/status"))
        .send()
        .await
        .expect("read proxy status")
        .text()
        .await
        .expect("proxy status body");
    assert!(
        !status_body.contains(BINDING_KEY),
        "proxy status leaked the binding key: {status_body}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn successful_upstream_body_echo_is_rejected_before_usage_or_observability() {
    const BINDING_KEY: &str = "upstream-success-echo-binding-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "credential-success-echo-probe",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("send successful upstream echo probe");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    assert!(response
        .headers()
        .get("x-upstream-credential-echo")
        .is_none());
    let response_body = response
        .text()
        .await
        .expect("successful echo rejection body");
    assert!(!response_body.contains(BINDING_KEY), "{response_body}");
    assert_eq!(
        serde_json::from_str::<Value>(&response_body).expect("generic 502 JSON"),
        json!({
            "error": {
                "message": "upstream response rejected",
                "type": "proxy_error"
            }
        })
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = db
        .list_usage_events("metered-e2e", 0, chrono::Utc::now().timestamp() + 2, 1, 20)
        .expect("query rejected-success usage events");
    assert_eq!(events.total, 0);
    let status_body = client
        .get(format!("http://127.0.0.1:{proxy_port}/status"))
        .send()
        .await
        .expect("read proxy status")
        .text()
        .await
        .expect("proxy status body");
    assert!(!status_body.contains(BINDING_KEY), "{status_body}");
    let exported_sql = db.export_sql_string().expect("export observability state");
    assert!(!exported_sql.contains(BINDING_KEY), "{exported_sql}");

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn upstream_redirect_is_not_followed_or_reflected_to_the_client() {
    const BINDING_KEY: &str = "redirect-protected-binding-key";

    let target_hits = Arc::new(AtomicUsize::new(0));
    let (redirect_target_address, redirect_target_shutdown) =
        start_redirect_target(target_hits.clone()).await;
    let redirect_location =
        format!("http://{redirect_target_address}/credential-capture?binding_key={BINDING_KEY}");
    let redirect_hits = Arc::new(AtomicUsize::new(0));
    let (redirecting_upstream, redirecting_upstream_shutdown) =
        start_redirecting_upstream(redirect_hits.clone(), redirect_location.clone()).await;

    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(redirecting_upstream))
        .expect("save redirecting provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build no-redirect local client");
    let response = send_message_with_bearer(&client, proxy_port, Some(BINDING_KEY)).await;
    assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    assert!(response.headers().get("location").is_none());
    assert!(response.headers().get("x-upstream-redirect").is_none());
    let response_body = response.text().await.expect("generic redirect error body");
    assert!(
        !response_body.contains(&redirect_location),
        "{response_body}"
    );
    assert!(!response_body.contains(BINDING_KEY), "{response_body}");
    assert_eq!(
        serde_json::from_str::<Value>(&response_body).expect("generic redirect JSON"),
        json!({
            "error": {
                "message": "Upstream error (status 302)",
                "type": "upstream_error"
            }
        })
    );
    assert_eq!(redirect_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        target_hits.load(Ordering::SeqCst),
        0,
        "the upstream client must not follow a credential-bearing redirect"
    );

    let events = db
        .list_usage_events("metered-e2e", 0, chrono::Utc::now().timestamp() + 2, 1, 20)
        .expect("query redirect usage events");
    assert_eq!(events.total, 0);
    let status_body = client
        .get(format!("http://127.0.0.1:{proxy_port}/status"))
        .send()
        .await
        .expect("read proxy status")
        .text()
        .await
        .expect("proxy status body");
    assert!(!status_body.contains(&redirect_location), "{status_body}");
    assert!(!status_body.contains(BINDING_KEY), "{status_body}");

    proxy.stop().await.expect("stop proxy");
    let _ = redirecting_upstream_shutdown.send(());
    let _ = redirect_target_shutdown.send(());
}

#[tokio::test]
async fn codex_rejects_binding_key_recontamination_outside_the_auth_header() {
    const BINDING_KEY: &str = "codex-recontamination-key/7";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_codex_provider(upstream))
        .expect("save Codex provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "codex", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");

    let safe = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "gpt-5",
            "input": "hello",
            "stream": false
        }))
        .send()
        .await
        .expect("send safe Codex request");
    assert_eq!(safe.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let rejected = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .query(&[("trace", BINDING_KEY)])
        .bearer_auth(BINDING_KEY)
        .header("x-debug", BINDING_KEY)
        .json(&json!({
            "model": BINDING_KEY,
            "input": "hello",
            "stream": false
        }))
        .send()
        .await
        .expect("send credential recontamination probe");
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
    let rejected_body = rejected.text().await.expect("Codex local rejection body");
    assert!(!rejected_body.contains(BINDING_KEY), "{rejected_body}");
    assert!(
        !rejected_body.contains("codex-recontamination-key%2F7"),
        "{rejected_body}"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "credential recontamination must be rejected before upstream"
    );

    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        captured_header(&requests[0], "authorization"),
        Some("Bearer codex-recontamination-key/7")
    );
    assert!(captured_header(&requests[0], "x-debug").is_none());

    let status_body = client
        .get(format!("http://127.0.0.1:{proxy_port}/status"))
        .send()
        .await
        .expect("read proxy status")
        .text()
        .await
        .expect("proxy status body");
    assert!(!status_body.contains(BINDING_KEY), "{status_body}");
    assert!(!db
        .export_sql_string()
        .expect("export SQL")
        .contains(BINDING_KEY));

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn claude_gemini_native_binding_injects_only_google_api_key_header() {
    const BINDING_KEY: &str = "gemini-native-protected-binding-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_gemini_native_provider(upstream))
        .expect("save Gemini-native Claude provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let _response = send_message_with_bearer(&client, proxy_port, Some(BINDING_KEY)).await;

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        captured_header(&requests[0], "x-goog-api-key"),
        Some(BINDING_KEY)
    );
    assert!(captured_header(&requests[0], "x-api-key").is_none());
    assert!(captured_header(&requests[0], "authorization").is_none());
    assert!(!format!("{:?}", requests[0].headers).contains("old-gemini-route-secret"));

    let rejected = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "gemini-2.5-pro",
            "max_tokens": 16,
            "stream": false,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {"type": BINDING_KEY}
                }]
            }]
        }))
        .send()
        .await
        .expect("send credential-bearing transform error probe");
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
    let rejected_body = rejected.text().await.expect("transform error body");
    assert!(!rejected_body.contains(BINDING_KEY), "{rejected_body}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let transform_error = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .json(&json!({
            "model": "gemini-2.5-pro",
            "max_tokens": 16,
            "stream": false,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {"type": "unsupported-image-source"}
                }]
            }]
        }))
        .send()
        .await
        .expect("send non-secret transform error probe");
    assert_eq!(
        transform_error.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let transform_error_body = transform_error.text().await.expect("transform error body");
    assert!(
        !transform_error_body.contains(BINDING_KEY),
        "{transform_error_body}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let status_body = client
        .get(format!("http://127.0.0.1:{proxy_port}/status"))
        .send()
        .await
        .expect("read proxy status")
        .text()
        .await
        .expect("proxy status body");
    assert!(!status_body.contains(BINDING_KEY), "{status_body}");
    let exported_sql = db.export_sql_string().expect("export persistence probe");
    assert!(!exported_sql.contains(BINDING_KEY));
    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn split_gemini_stream_key_is_neither_exposed_cached_nor_ingested() {
    const BINDING_KEY: &str = "gemini-split-stream-binding-key";
    const SESSION_ID: &str = "gemini-split-shadow-session";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_gemini_native_provider(upstream))
        .expect("save Gemini-native Claude provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let split_response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .header("x-claude-code-session-id", SESSION_ID)
        .json(&json!({
            "model": "gemini-2.5-pro",
            "max_tokens": 16,
            "stream": true,
            "messages": [{"role": "user", "content": "split-secret-stream-probe"}]
        }))
        .send()
        .await;
    let mut visible_bytes = Vec::new();
    let mut stream_rejected = false;
    match split_response {
        Ok(mut response) => {
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => visible_bytes.extend_from_slice(&chunk),
                    Ok(None) => break,
                    Err(_) => {
                        stream_rejected = true;
                        break;
                    }
                }
            }
        }
        Err(error) => {
            stream_rejected = true;
            visible_bytes.extend_from_slice(error.to_string().as_bytes());
        }
    }
    assert!(
        stream_rejected,
        "the split credential stream must be aborted"
    );
    let visible_stream = String::from_utf8_lossy(&visible_bytes);
    let (first_half, second_half) = BINDING_KEY.split_at(BINDING_KEY.len() / 2);
    assert!(!visible_stream.contains(BINDING_KEY), "{visible_stream}");
    assert!(
        !visible_stream.contains(first_half) && !visible_stream.contains(second_half),
        "no credential fragment may be released before the semantic stream guard rejects it: {visible_stream}"
    );

    let followup = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth(BINDING_KEY)
        .header("x-claude-code-session-id", SESSION_ID)
        .json(&json!({
            "model": "gemini-2.5-pro",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "shadow-followup-probe"}]
        }))
        .send()
        .await
        .expect("send same-session shadow followup");
    assert_eq!(followup.status(), reqwest::StatusCode::OK);
    let followup_body = followup.text().await.expect("shadow followup body");
    assert!(!followup_body.contains(BINDING_KEY), "{followup_body}");

    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].uri.contains(":streamGenerateContent"));
    assert!(requests[0].uri.contains("alt=sse"));
    assert!(requests[1].uri.contains(":generateContent"));
    assert!(!requests[1].uri.contains(":streamGenerateContent"));
    let followup_upstream_body = requests[1].body.to_string();
    assert!(
        !followup_upstream_body.contains(BINDING_KEY),
        "the rejected stream must not contaminate the Gemini shadow replay: {followup_upstream_body}"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = db
        .list_usage_events("metered-e2e", 0, chrono::Utc::now().timestamp() + 2, 1, 20)
        .expect("query split-stream usage events");
    assert_eq!(events.total, 0);
    assert!(!db
        .export_sql_string()
        .expect("export split-stream persistence probe")
        .contains(BINDING_KEY));

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn gemini_setup_base_path_query_binding_is_stripped_and_frozen_to_the_gemini_agent() {
    const BINDING_KEY: &str = "gemini-query-protected-binding-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_gemini_provider(upstream))
        .expect("save Gemini provider");
    let gemini_agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "Gemini ingress E2E".to_string(),
            sort_order: 50,
            visible: true,
        })
        .expect("create Gemini ingress Agent");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, &gemini_agent.id, BINDING_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;
    let response = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/gemini/v1beta/models/gemini-2.5-pro:generateContent?key={BINDING_KEY}&trace=safe"
        ))
        .json(&json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .expect("send Gemini query-key request through proxy");
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let events = wait_for_events(&db, start_at, chrono::Utc::now().timestamp() + 10, 1).await;
    assert_eq!(
        events.items[0].agent_module_id.as_deref(),
        Some(gemini_agent.id.as_str())
    );
    assert_eq!(events.items[0].provider_id, "metered-e2e");
    assert_eq!(events.items[0].model, "gemini-2.5-pro");

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].uri,
        "/v1beta/models/gemini-2.5-pro:generateContent?trace=safe"
    );
    assert_eq!(
        captured_header(&requests[0], "x-goog-api-key"),
        Some(BINDING_KEY)
    );
    assert!(captured_header(&requests[0], "x-api-key").is_none());
    assert!(captured_header(&requests[0], "authorization").is_none());
    let serialized = format!(
        "{:?}\n{}\n{}",
        requests[0].headers, requests[0].uri, requests[0].body
    );
    assert!(!serialized.contains("old-gemini-route-key"));

    let rejected = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/v1beta/models/{BINDING_KEY}:generateContent"
        ))
        .header("x-goog-api-key", BINDING_KEY)
        .json(&json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .expect("send credential-bearing Gemini path probe");
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
    let rejected_body = rejected.text().await.expect("Gemini path error body");
    assert!(!rejected_body.contains(BINDING_KEY), "{rejected_body}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

async fn send_rectifier_retry_probe(
    client: &reqwest::Client,
    port: u16,
    binding_key: &str,
) -> reqwest::Response {
    client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .bearer_auth(binding_key)
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

fn captured_header<'a>(request: &'a CapturedUpstreamRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn assert_generic_local_rejection(response_body: &str, rejected_key: &str) {
    assert!(!response_body.contains(rejected_key));
    for internal_reason in [
        "binding_not_found",
        "invalid_binding",
        "credential_unavailable",
        "credential_required",
        "protocol mismatch",
        "archived",
        "disabled",
    ] {
        assert!(
            !response_body.contains(internal_reason),
            "public rejection leaked internal reason {internal_reason}: {response_body}"
        );
    }
}

fn assert_public_snapshots_omit_binding_keys(
    db: &Database,
    start_at: i64,
    end_at: i64,
    binding_keys: &[&str],
) {
    let public_snapshot = json!({
        "events": db
            .list_usage_events("metered-e2e", start_at, end_at, 1, 20)
            .expect("query public usage events"),
        "diagnostics": db
            .get_unassigned_usage_diagnostics()
            .expect("query public diagnostics"),
    })
    .to_string();
    let sql_snapshot = db.export_sql_string().expect("export database snapshot");

    for binding_key in binding_keys {
        assert!(
            !public_snapshot.contains(binding_key),
            "public snapshot retained a binding key"
        );
        assert!(
            !sql_snapshot.contains(binding_key),
            "database export retained a binding key"
        );
    }
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
async fn two_agent_keys_share_a_provider_but_freeze_distinct_event_ownership() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let claude_binding = create_enabled_binding(
        &db,
        &credentials,
        "claude-code",
        "claude-agent-protected-key",
    )
    .await;
    create_enabled_binding(&db, &credentials, "codex", "codex-agent-protected-key").await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    let claude = send_message_with_bearer_and_query(
        &client,
        proxy_port,
        Some("claude-agent-protected-key"),
        Some("token=inbound-query-secret&trace=safe"),
    )
    .await;
    assert_eq!(claude.status(), reqwest::StatusCode::OK);
    let codex =
        send_message_with_bearer(&client, proxy_port, Some("codex-agent-protected-key")).await;
    assert_eq!(codex.status(), reqwest::StatusCode::OK);

    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    let claude_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-e2e-0"))
        .expect("Claude-owned event");
    assert_eq!(claude_event.agent_module_id.as_deref(), Some("claude-code"));
    let frozen_claude_event = claude_event.clone();
    let codex_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-e2e-1"))
        .expect("Codex-owned event");
    assert_eq!(codex_event.agent_module_id.as_deref(), Some("codex"));
    assert!(events
        .items
        .iter()
        .all(|event| event.provider_id == "metered-e2e"));

    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].uri, "/v1/messages?trace=safe");
    assert_eq!(requests[1].uri, "/v1/messages");
    for (request, protected_key) in requests
        .iter()
        .zip(["claude-agent-protected-key", "codex-agent-protected-key"])
    {
        assert_eq!(captured_header(request, "x-api-key"), Some(protected_key));
        assert!(captured_header(request, "authorization").is_none());
        let serialized = format!("{:?}\n{}\n{}", request.headers, request.uri, request.body);
        for forbidden in [
            "old-route-config-secret",
            "old-legacy-envelope-secret",
            "inbound-query-secret",
            "inbound-proxy-auth-secret",
            "inbound-api-key-secret",
            "inbound-auth-token-secret",
            "inbound-aws-token-secret",
            "inbound-cookie-secret",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "upstream request leaked {forbidden}: {serialized}"
            );
        }
    }

    credentials
        .delete_binding(&claude_binding.id, claude_binding.credential_version)
        .await
        .expect("delete the original Claude binding");
    let rebound_agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "Rebound ownership E2E".to_string(),
            sort_order: 50,
            visible: true,
        })
        .expect("create replacement Agent");
    create_enabled_binding(
        &db,
        &credentials,
        &rebound_agent.id,
        "rebound-agent-protected-key",
    )
    .await;
    let rebound =
        send_message_with_bearer(&client, proxy_port, Some("rebound-agent-protected-key")).await;
    assert_eq!(rebound.status(), reqwest::StatusCode::OK);

    let rebound_events = wait_for_events(&db, start_at, end_at, 3).await;
    let preserved = rebound_events
        .items
        .iter()
        .find(|event| event.event_id == frozen_claude_event.event_id)
        .expect("preserved pre-rebind event");
    assert_eq!(preserved, &frozen_claude_event);
    let replacement = rebound_events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-e2e-2"))
        .expect("replacement Agent event");
    assert_eq!(
        replacement.agent_module_id.as_deref(),
        Some(rebound_agent.id.as_str())
    );
    assert_eq!(replacement.provider_id, "metered-e2e");
    assert_public_snapshots_omit_binding_keys(
        &db,
        start_at,
        end_at,
        &[
            "claude-agent-protected-key",
            "codex-agent-protected-key",
            "rebound-agent-protected-key",
        ],
    );
    assert_app_logs_omit_binding_keys(&[
        "claude-agent-protected-key",
        "codex-agent-protected-key",
        "rebound-agent-protected-key",
    ]);
    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn codex_two_binding_keys_freeze_distinct_agent_ownership() {
    const CODEX_KEY: &str = "codex-two-key-acceptance";
    const OPENCODE_KEY: &str = "opencode-two-key-acceptance";
    const ROTATED_CODEX_KEY: &str = "codex-rotated-key-acceptance";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_codex_provider(upstream))
        .expect("save Codex provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let codex_binding = create_enabled_binding(&db, &credentials, "codex", CODEX_KEY).await;
    let opencode_binding =
        create_enabled_binding(&db, &credentials, "opencode", OPENCODE_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    for key in [CODEX_KEY, OPENCODE_KEY] {
        let response = client
            .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
            .bearer_auth(key)
            .json(&json!({
                "model": "gpt-5",
                "input": "hello",
                "stream": false
            }))
            .send()
            .await
            .expect("send Codex request through proxy");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    let codex_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("resp-e2e-0"))
        .expect("Codex-owned event");
    assert_eq!(codex_event.agent_module_id.as_deref(), Some("codex"));
    let opencode_event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("resp-e2e-1"))
        .expect("OpenCode-owned event");
    assert_eq!(opencode_event.agent_module_id.as_deref(), Some("opencode"));
    assert!(events
        .items
        .iter()
        .all(|event| event.provider_id == "metered-e2e"));
    let frozen_events = events.items.clone();

    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 2);
    for (request, expected_key) in requests.iter().zip([CODEX_KEY, OPENCODE_KEY]) {
        let expected_authorization = format!("Bearer {expected_key}");
        assert_eq!(request.uri, "/v1/responses");
        assert_eq!(
            captured_header(request, "authorization"),
            Some(expected_authorization.as_str())
        );
        assert!(captured_header(request, "x-api-key").is_none());
        assert!(captured_header(request, "x-goog-api-key").is_none());
        for binding_key in [CODEX_KEY, OPENCODE_KEY] {
            assert!(!request.uri.contains(binding_key));
            assert!(!request.body.to_string().contains(binding_key));
        }
    }

    credentials
        .replace_binding_api_key(
            &codex_binding.id,
            codex_binding.credential_version,
            SecretString::new(ROTATED_CODEX_KEY.to_string()),
        )
        .await
        .expect("rotate Codex binding key");
    credentials
        .delete_binding(&opencode_binding.id, opencode_binding.credential_version)
        .await
        .expect("delete OpenCode binding");

    let preserved = db
        .list_usage_events("metered-e2e", start_at, end_at, 1, 20)
        .expect("query preserved Codex history");
    assert_eq!(preserved.items.len(), frozen_events.len());
    for frozen in &frozen_events {
        assert_eq!(
            preserved
                .items
                .iter()
                .find(|event| event.event_id == frozen.event_id),
            Some(frozen)
        );
    }
    assert_public_snapshots_omit_binding_keys(
        &db,
        start_at,
        end_at,
        &[CODEX_KEY, OPENCODE_KEY, ROTATED_CODEX_KEY],
    );
    assert_app_logs_omit_binding_keys(&[CODEX_KEY, OPENCODE_KEY, ROTATED_CODEX_KEY]);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn gemini_two_binding_keys_freeze_distinct_agent_ownership() {
    const HERMES_KEY: &str = "hermes-gemini-two-key-acceptance";
    const OPENCLAW_KEY: &str = "openclaw-gemini-two-key-acceptance";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_gemini_provider(upstream))
        .expect("save Gemini provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "hermes", HERMES_KEY).await;
    create_enabled_binding(&db, &credentials, "openclaw", OPENCLAW_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    for (key, trace) in [(HERMES_KEY, "hermes"), (OPENCLAW_KEY, "openclaw")] {
        let response = client
            .post(format!(
                "http://127.0.0.1:{proxy_port}/v1beta/models/gemini-2.5-pro:generateContent?key={key}&trace={trace}"
            ))
            .json(&json!({
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
            }))
            .send()
            .await
            .expect("send Gemini request through proxy");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    let hermes_event = events
        .items
        .iter()
        .find(|event| event.input_tokens == 7 && event.output_tokens == 3)
        .expect("Hermes-owned Gemini event");
    assert_eq!(hermes_event.agent_module_id.as_deref(), Some("hermes"));
    let openclaw_event = events
        .items
        .iter()
        .find(|event| event.input_tokens == 11 && event.output_tokens == 5)
        .expect("OpenClaw-owned Gemini event");
    assert_eq!(openclaw_event.agent_module_id.as_deref(), Some("openclaw"));
    assert!(events
        .items
        .iter()
        .all(|event| event.provider_id == "metered-e2e"));

    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 2);
    for ((request, expected_key), trace) in requests
        .iter()
        .zip([HERMES_KEY, OPENCLAW_KEY])
        .zip(["hermes", "openclaw"])
    {
        assert_eq!(
            request.uri,
            format!("/v1beta/models/gemini-2.5-pro:generateContent?trace={trace}")
        );
        assert_eq!(
            captured_header(request, "x-goog-api-key"),
            Some(expected_key)
        );
        assert!(captured_header(request, "authorization").is_none());
        assert!(captured_header(request, "x-api-key").is_none());
        for binding_key in [HERMES_KEY, OPENCLAW_KEY] {
            assert!(!request.uri.contains(binding_key));
            assert!(!request.body.to_string().contains(binding_key));
        }
    }
    assert_public_snapshots_omit_binding_keys(&db, start_at, end_at, &[HERMES_KEY, OPENCLAW_KEY]);
    assert_app_logs_omit_binding_keys(&[HERMES_KEY, OPENCLAW_KEY]);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn rotating_a_binding_key_revokes_the_old_key_before_the_next_upstream_request() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let binding =
        create_enabled_binding(&db, &credentials, "claude-code", "rotation-old-key").await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;
    let old_before_rotation =
        send_message_with_bearer(&client, proxy_port, Some("rotation-old-key")).await;
    assert_eq!(old_before_rotation.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let rotated = credentials
        .replace_binding_api_key(
            &binding.id,
            binding.credential_version,
            SecretString::new("rotation-new-key".to_string()),
        )
        .await
        .expect("rotate binding key");
    assert_eq!(
        rotated.credential_version,
        binding.credential_version + 1,
        "rotation advances the binding generation"
    );

    let revoked = send_message_with_bearer(&client, proxy_port, Some("rotation-old-key")).await;
    assert_eq!(revoked.status(), reqwest::StatusCode::UNAUTHORIZED);
    let revoked_body = revoked.text().await.expect("revoked-key response body");
    assert_generic_local_rejection(&revoked_body, "rotation-old-key");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let replacement = send_message_with_bearer(&client, proxy_port, Some("rotation-new-key")).await;
    assert_eq!(replacement.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 2).await;
    assert!(events
        .items
        .iter()
        .all(|event| event.agent_module_id.as_deref() == Some("claude-code")));

    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 2, "revoked key never reached upstream");
    assert_eq!(
        captured_header(&requests[0], "x-api-key"),
        Some("rotation-old-key")
    );
    assert_eq!(
        captured_header(&requests[1], "x-api-key"),
        Some("rotation-new-key")
    );
    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn rotation_completed_during_resolution_blocks_the_old_key_before_upstream() {
    const OLD_KEY: &str = "rotation-barrier-old-key";
    const NEW_KEY: &str = "rotation-barrier-new-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let binding = create_enabled_binding(&db, &credentials, "claude-code", OLD_KEY).await;

    let (proxy, proxy_port) = start_proxy(db, store.clone()).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    // The first matching protected-store read is the early authorization
    // preflight. Pause the second match so the mutation races the authoritative
    // resolve read immediately before request construction.
    store.arm_get_barrier_after_matches(OLD_KEY, 1);
    let old_request = tokio::spawn({
        let client = client.clone();
        async move { send_message_with_bearer(&client, proxy_port, Some(OLD_KEY)).await }
    });

    let barrier_reached = tokio::time::timeout(Duration::from_secs(5), async {
        while !store.get_barrier_captured() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if barrier_reached.is_err() {
        store.release_get_barrier();
        panic!("request did not reach the protected-store resolution barrier");
    }

    let rotated = credentials
        .replace_binding_api_key(
            &binding.id,
            binding.credential_version,
            SecretString::new(NEW_KEY.to_string()),
        )
        .await
        .expect("rotate while the old credential read is paused");
    assert_eq!(rotated.credential_version, binding.credential_version + 1);
    store.release_get_barrier();

    let stale_response = old_request.await.expect("join paused old-key request");
    assert_eq!(stale_response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let stale_body = stale_response.text().await.expect("stale response body");
    assert_generic_local_rejection(&stale_body, OLD_KEY);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "an old credential resolved before rotation must be revalidated before upstream"
    );
    assert!(captured.lock().expect("captured requests lock").is_empty());

    let replacement = send_message_with_bearer(&client, proxy_port, Some(NEW_KEY)).await;
    assert_eq!(replacement.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn resolved_stream_survives_rebind_and_ingests_with_frozen_ownership() {
    const OLD_KEY: &str = "frozen-stream-old-key";
    const REPLACEMENT_KEY: &str = "replacement-stream-key";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let stream_gate = Arc::new(StreamResponseGate::default());
    let (upstream, upstream_shutdown) = start_mock_upstream_with_stream_gate(
        hits.clone(),
        captured.clone(),
        Some(stream_gate.clone()),
    )
    .await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save original metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let original_binding = create_enabled_binding(&db, &credentials, "claude-code", OLD_KEY).await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;
    let in_flight = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
                .bearer_auth(OLD_KEY)
                .json(&json!({
                    "model": "frozen-stream-probe",
                    "max_tokens": 16,
                    "stream": true,
                    "messages": [{"role": "user", "content": "hello"}]
                }))
                .send()
                .await
                .expect("send in-flight streaming request")
        }
    });

    tokio::time::timeout(Duration::from_secs(5), stream_gate.wait_until_reached())
        .await
        .expect("stream request crossed the final binding resolution boundary");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let requests_before_rebind = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests_before_rebind.len(), 1);
    assert_eq!(requests_before_rebind[0].uri, "/v1/messages");
    assert_eq!(requests_before_rebind[0].body["stream"], true);
    assert_eq!(
        captured_header(&requests_before_rebind[0], "x-api-key"),
        Some(OLD_KEY),
        "the request that crossed resolution must retain the old protected key"
    );

    credentials
        .delete_binding(&original_binding.id, original_binding.credential_version)
        .await
        .expect("delete the binding after its request reached upstream");
    let replacement_agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "In-flight replacement Agent".to_string(),
            sort_order: 50,
            visible: true,
        })
        .expect("create replacement Agent");
    let mut replacement_provider = metered_provider(upstream);
    replacement_provider.id = "replacement-e2e".to_string();
    replacement_provider.name = "Replacement E2E".to_string();
    replacement_provider.product_group_id = "replacement-product".to_string();
    replacement_provider.route_config = Some(json!({
        "baseUrl": "http://127.0.0.1:9",
        "apiFormat": "anthropic"
    }));
    db.save_usage_provider(&replacement_provider)
        .expect("save replacement provider");
    create_enabled_binding_for_provider(
        &db,
        &credentials,
        &replacement_agent.id,
        "replacement-e2e",
        REPLACEMENT_KEY,
    )
    .await;

    stream_gate.release();
    let response = in_flight.await.expect("join in-flight request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream")));
    let stream_body = response.text().await.expect("consume streamed response");
    assert!(stream_body.contains("event: message_start"));
    assert!(stream_body.contains("event: message_stop"));

    let end_at = chrono::Utc::now().timestamp() + 10;
    let events = wait_for_events(&db, start_at, end_at, 1).await;
    let event = events
        .items
        .iter()
        .find(|event| event.upstream_correlation_id.as_deref() == Some("msg-stream-frozen-0"))
        .expect("asynchronously ingested streaming event");
    assert_eq!(event.agent_module_id.as_deref(), Some("claude-code"));
    assert_eq!(event.provider_id, "metered-e2e");
    assert_eq!(event.model, "claude-sonnet-4-5-20250929");
    assert_eq!(event.input_tokens, 11);
    assert_eq!(event.output_tokens, 5);
    assert_eq!(event.cache_read_tokens, 2);
    assert_eq!(event.cache_creation_tokens, 3);
    assert_eq!(event.total_cost_usd.as_deref(), Some("0.17"));
    assert_eq!(event.cost_source, CostSource::Upstream);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        captured.lock().expect("captured requests lock").len(),
        1,
        "rebinding while the stream is in flight must not produce another upstream attempt"
    );

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn archiving_an_agent_after_success_blocks_new_hits_and_preserves_frozen_history() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let archived_agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "Archive after success".to_string(),
            sort_order: 50,
            visible: true,
        })
        .expect("create archivable agent");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    let binding = create_enabled_binding(
        &db,
        &credentials,
        &archived_agent.id,
        "archive-after-success-key",
    )
    .await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;
    let accepted =
        send_message_with_bearer(&client, proxy_port, Some("archive-after-success-key")).await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    let end_at = chrono::Utc::now().timestamp() + 10;
    let before_archive = wait_for_events(&db, start_at, end_at, 1).await;
    assert_eq!(
        before_archive.items[0].agent_module_id.as_deref(),
        Some(archived_agent.id.as_str())
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    db.delete_agent_module(&archived_agent.id)
        .expect("archive agent with history");
    let rejected =
        send_message_with_bearer(&client, proxy_port, Some("archive-after-success-key")).await;
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
    let rejected_body = rejected
        .text()
        .await
        .expect("archived-agent rejection body");
    assert_generic_local_rejection(&rejected_body, "archive-after-success-key");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let after_archive = db
        .list_usage_events("metered-e2e", start_at, end_at, 1, 20)
        .expect("query preserved usage history");
    assert_eq!(after_archive.total, 1);
    assert_eq!(
        after_archive.items[0].agent_module_id.as_deref(),
        Some(archived_agent.id.as_str())
    );

    credentials
        .clear_binding_api_key(&binding.id, binding.credential_version)
        .await
        .expect("clean protected slot after Agent archive");
    let rejected_after_cleanup =
        send_message_with_bearer(&client, proxy_port, Some("archive-after-success-key")).await;
    assert_eq!(
        rejected_after_cleanup.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let rejected_after_cleanup_body = rejected_after_cleanup
        .text()
        .await
        .expect("post-cleanup archived-agent rejection body");
    assert_generic_local_rejection(&rejected_after_cleanup_body, "archive-after-success-key");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let after_cleanup = db
        .list_usage_events("metered-e2e", start_at, end_at, 1, 20)
        .expect("query history after protected-slot cleanup");
    assert_eq!(after_cleanup.total, 1);
    assert_eq!(
        after_cleanup.items[0].agent_module_id.as_deref(),
        Some(archived_agent.id.as_str())
    );

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn claude_desktop_models_never_falls_back_to_a_static_compatibility_route() {
    const LEGACY_SENTINEL: &str = "legacy-desktop-meta-secret-sentinel";

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let temp = tempfile::tempdir().expect("Desktop E2E temp directory");
    let database_path = temp.path().join("desktop-e2e.sqlite");
    let db = Arc::new(Database::init_at(&database_path).expect("Desktop E2E database"));
    let mut legacy_provider = Provider::with_id(
        "legacy-desktop-provider".to_string(),
        "Legacy Desktop Provider".to_string(),
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("http://{upstream}"),
                "ANTHROPIC_API_KEY": LEGACY_SENTINEL
            }
        }),
        None,
    );
    legacy_provider.meta = Some(
        serde_json::from_value(json!({
            "claudeDesktopModelRoutes": {
                "claude-sonnet-4-6": {
                    "model": LEGACY_SENTINEL,
                    "labelOverride": "Legacy route must be ignored"
                }
            },
            "customUserAgent": format!("legacy-agent-{LEGACY_SENTINEL}"),
            "promptCacheKey": format!("legacy-cache-{LEGACY_SENTINEL}"),
            "localProxyRequestOverrides": {
                "headers": {"X-Relay-Auth": LEGACY_SENTINEL},
                "body": {"legacyRelayValue": LEGACY_SENTINEL}
            }
        }))
        .expect("legacy Desktop meta"),
    );
    db.save_provider("claude-desktop", &legacy_provider)
        .expect("save legacy Desktop provider");
    let mut authoritative_provider = metered_provider(upstream);
    authoritative_provider.route_app_type = Some("claude-desktop".to_string());
    authoritative_provider.route_config = Some(json!({
        "baseUrl": format!("http://{upstream}"),
        "apiFormat": "anthropic",
        "authMode": "x-api-key",
        "claudeDesktopModelRoutes": {
            "claude-sonnet-4-6": {
                "model": "claude-sonnet-4-5-20250929",
                "labelOverride": "Desktop Sonnet"
            }
        }
    }));
    db.save_usage_provider(&authoritative_provider)
        .expect("save compatibility provider");
    rusqlite::Connection::open(&database_path)
        .expect("open Desktop E2E link connection")
        .execute(
            "UPDATE usage_providers
             SET legacy_app_type = 'claude-desktop', legacy_provider_id = ?1
             WHERE id = 'metered-e2e'",
            [legacy_provider.id.as_str()],
        )
        .expect("link usage provider to legacy Desktop provider");
    db.set_route_binding("claude", "metered-e2e")
        .expect("install dormant ordinary-Claude compatibility route");
    db.set_setting("claude_desktop_gateway_token", "desktop-gateway-token")
        .expect("install deterministic Desktop gateway token");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(
        &db,
        &credentials,
        "claude-code",
        "known-desktop-binding-key",
    )
    .await;
    let (proxy, proxy_port) = start_proxy(db, store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");

    let response = client
        .get(format!(
            "http://127.0.0.1:{proxy_port}/claude-desktop/v1/models"
        ))
        .bearer_auth("desktop-gateway-token")
        .header("x-api-key", "unknown-desktop-binding-key")
        .send()
        .await
        .expect("request Desktop model catalog");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body = response.text().await.expect("Desktop rejection body");
    assert_generic_local_rejection(&body, "unknown-desktop-binding-key");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(captured.lock().expect("captured requests lock").is_empty());

    let known = client
        .get(format!(
            "http://127.0.0.1:{proxy_port}/claude-desktop/v1/models"
        ))
        .bearer_auth("desktop-gateway-token")
        .header("x-api-key", "known-desktop-binding-key")
        .send()
        .await
        .expect("request Desktop model catalog with a known binding");
    assert_eq!(known.status(), reqwest::StatusCode::OK);
    let catalog: Value = known.json().await.expect("Desktop model catalog JSON");
    assert_eq!(catalog["data"][0]["id"], "claude-sonnet-4-6");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(captured.lock().expect("captured requests lock").is_empty());

    let ordinary_claude =
        send_message_with_bearer(&client, proxy_port, Some("known-desktop-binding-key")).await;
    assert_eq!(ordinary_claude.status(), reqwest::StatusCode::UNAUTHORIZED);
    let ordinary_body = ordinary_claude
        .text()
        .await
        .expect("ordinary Claude protocol-mismatch body");
    assert_generic_local_rejection(&ordinary_body, "known-desktop-binding-key");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "a Desktop-only binding must never reach the ordinary Claude route"
    );
    assert!(captured.lock().expect("captured requests lock").is_empty());

    let message = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/claude-desktop/v1/messages"
        ))
        .bearer_auth("desktop-gateway-token")
        .header("x-api-key", "known-desktop-binding-key")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 16,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("send Desktop message with distinct gateway and binding credentials");
    assert_eq!(message.status(), reqwest::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let requests = captured.lock().expect("captured requests lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        captured_header(&requests[0], "x-api-key"),
        Some("known-desktop-binding-key")
    );
    assert!(captured_header(&requests[0], "authorization").is_none());
    assert_eq!(requests[0].body["model"], "claude-sonnet-4-5-20250929");
    let serialized = format!(
        "{:?}\n{}\n{}",
        requests[0].headers, requests[0].uri, requests[0].body
    );
    assert!(!serialized.contains(LEGACY_SENTINEL));

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn unusable_bindings_fail_with_a_generic_local_401_without_usage_events() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());

    let disabled =
        create_enabled_binding(&db, &credentials, "claude-code", "disabled-binding-key").await;
    db.save_agent_provider_binding(&AgentProviderBindingInput {
        id: Some(disabled.id),
        agent_module_id: "claude-code".to_string(),
        provider_id: "metered-e2e".to_string(),
        enabled: false,
    })
    .expect("disable binding");

    let cleared =
        create_enabled_binding(&db, &credentials, "opencode", "cleared-binding-key").await;
    credentials
        .clear_binding_api_key(&cleared.id, cleared.credential_version)
        .await
        .expect("clear binding key");

    create_enabled_binding(&db, &credentials, "openclaw", "protected-store-missing-key").await;
    store.remove_secret_value("protected-store-missing-key");

    create_enabled_binding(&db, &credentials, "codex", "protocol-mismatch-binding-key").await;

    let archived_agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "Archived proxy agent".to_string(),
            sort_order: 50,
            visible: true,
        })
        .expect("create archivable agent");
    create_enabled_binding(
        &db,
        &credentials,
        &archived_agent.id,
        "archived-agent-binding-key",
    )
    .await;
    db.delete_agent_module(&archived_agent.id)
        .expect("archive bound agent");

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let mut claude_rejection_bodies = Vec::new();
    let missing = send_message_with_bearer(&client, proxy_port, None).await;
    assert_eq!(missing.status(), reqwest::StatusCode::UNAUTHORIZED);
    let missing_body = missing.text().await.expect("missing-key rejection body");
    assert_generic_local_rejection(&missing_body, "missing-key-was-never-sent");

    let malformed_store_missing = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .bearer_auth("protected-store-missing-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"unterminated"#)
        .send()
        .await
        .expect("send malformed request with a store-missing binding key");
    assert_eq!(
        malformed_store_missing.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let malformed_store_missing_body = malformed_store_missing
        .text()
        .await
        .expect("malformed store-missing rejection body");
    assert_eq!(
        malformed_store_missing_body, missing_body,
        "preflight must reject a store-missing key before parsing and expose the exact generic body"
    );
    assert_generic_local_rejection(&malformed_store_missing_body, "protected-store-missing-key");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(captured.lock().expect("captured requests lock").is_empty());

    claude_rejection_bodies.push(missing_body);
    claude_rejection_bodies.push(malformed_store_missing_body);
    for key in [
        "disabled-binding-key",
        "cleared-binding-key",
        "protected-store-missing-key",
        "archived-agent-binding-key",
    ] {
        let response = send_message_with_bearer(&client, proxy_port, Some(key)).await;
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        let body = response
            .text()
            .await
            .expect("generic Claude rejection body");
        assert_generic_local_rejection(&body, key);
        claude_rejection_bodies.push(body);
    }
    assert!(claude_rejection_bodies
        .windows(2)
        .all(|pair| pair[0] == pair[1]));

    let protocol_mismatch = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .bearer_auth("protocol-mismatch-binding-key")
        .json(&json!({"model": "gpt-5", "input": "hello"}))
        .send()
        .await
        .expect("send protocol mismatch request");
    assert_eq!(
        protocol_mismatch.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let mismatch_body = protocol_mismatch
        .text()
        .await
        .expect("generic protocol rejection body");
    assert_generic_local_rejection(&mismatch_body, "protocol-mismatch-binding-key");

    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(captured.lock().expect("captured requests lock").is_empty());
    let end_at = chrono::Utc::now().timestamp() + 2;
    let events = db
        .list_usage_events("metered-e2e", 0, end_at, 1, 20)
        .expect("query usage events");
    assert_eq!(events.total, 0);

    proxy.stop().await.expect("stop proxy");
    let _ = upstream_shutdown.send(());
}

#[tokio::test]
async fn real_proxy_requests_feed_dashboard_and_route_errors_stay_local() {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (upstream, upstream_shutdown) = start_mock_upstream(hits.clone(), captured.clone()).await;
    let db = Arc::new(Database::memory().expect("in-memory database"));
    db.save_usage_provider(&metered_provider(upstream))
        .expect("save metered provider");
    db.set_route_binding("claude", "metered-e2e")
        .expect("install dormant compatibility route");
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = BindingCredentialService::new(db.clone(), store.clone());
    create_enabled_binding(&db, &credentials, "claude-code", "dashboard-binding-key").await;

    let (proxy, proxy_port) = start_proxy(db.clone(), store).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local client");
    let start_at = chrono::Utc::now().timestamp() - 2;

    let explicit =
        send_message_with_bearer(&client, proxy_port, Some("dashboard-binding-key")).await;
    assert_eq!(explicit.status(), reqwest::StatusCode::OK);
    let explicit_body: Value = explicit.json().await.expect("explicit upstream response");
    assert_eq!(explicit_body["id"], "msg-e2e-0");

    let estimated =
        send_message_with_bearer(&client, proxy_port, Some("dashboard-binding-key")).await;
    assert_eq!(estimated.status(), reqwest::StatusCode::OK);
    let estimated_body: Value = estimated.json().await.expect("estimated upstream response");
    assert_eq!(estimated_body["id"], "msg-e2e-1");

    let hits_before_probe = hits.load(Ordering::SeqCst);
    let retry_probe =
        send_rectifier_retry_probe(&client, proxy_port, "dashboard-binding-key").await;
    assert_eq!(retry_probe.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        hits_before_probe + 1,
        "the resolved binding route must make exactly one upstream attempt"
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
    assert_eq!(
        upstream_event.agent_module_id.as_deref(),
        Some("claude-code")
    );
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
    assert_eq!(
        estimated_event.agent_module_id.as_deref(),
        Some("claude-code")
    );
    assert_eq!(estimated_event.cost_source, CostSource::Estimated);
    assert_eq!(estimated_event.total_cost_usd.as_deref(), Some("0.00012"));

    let dashboard = UsageDashboardService::new(&db)
        .get_dashboard(start_at, end_at, "claude-code")
        .expect("query usage dashboard");
    assert_eq!(dashboard.agent_module_id, "claude-code");
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

    proxy.stop().await.expect("stop bound proxy");
    let _ = upstream_shutdown.send(());
}
