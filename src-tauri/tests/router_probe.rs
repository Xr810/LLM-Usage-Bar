//! T21 验证任务的端到端回归探针(#[ignore] 集成测试,平时 cargo test 不跑)。
//!
//! 任务书:docs/tasks/T21-first-real-run.md。这不是产品代码,是验证路径:
//! 让 router 真的处理一次请求。全部走真实入口:
//!
//! - `route::server::start()` 真实监听真实端口(绝不自己拼简化版);
//! - 上游认证走真实的 `RouterUpstreamAuth`(T9 生产实现);
//! - 配置播种走真实的 `RouterApi`(将来前端调的就是它);
//! - 记账落进真实文件库的 `router_attempts` 表,验完用 SQL 直接查表。
//!
//! 跑法(仓库约定:cargo 一律走 pnpm rust 包装器):
//!
//! ```bash
//! pnpm rust -- test --manifest-path src-tauri/Cargo.toml -- --ignored router_end_to_end_probe
//! ```
//!
//! 默认跑「本地假上游」三个阶段的全部断言:真实 start()、候选队列/改写/注入
//! 凭据/故障转移/拉黑/401 透传、router_attempts 与 token 回填、真实 Codex CLI
//! 打进 router(临时 CODEX_HOME,不碰真实 ~/.codex)。零外部网络、零花费。
//!
//! 两个可选阶段,默认关闭,经环境变量开启:
//!
//! - `T21_PROBE_MIGRATE_FROM=<某旧版 db 文件的路径>`:把该文件复制到临时目录后
//!   走 `Database::init_at` 的真实迁移链,逐表比对行数。**请指向你自己复制出来
//!   的副本,不要指线上库**。
//! - `T21_PROBE_REAL_TOKEN=<真实上游 token>`(可选 `T21_PROBE_REAL_BASE_URL`、
//!   `T21_PROBE_REAL_MODEL`):对真实上游发**一个**最小请求(真花钱,只在
//!   用户明确同意后使用)。token 只经环境变量进入,不落任何文件与日志。
//!
//! 红线(任务书 §1)在本探针里是结构性保证:
//! - 测试开头调 `support::ensure_test_home()`,HOME 与 LLM_USAGE_BAR_TEST_HOME
//!   都指向隔离目录,`get_home_dir()` 绝不落到真实 home;
//! - 迁移阶段只打开「环境变量指定的文件复制出来的副本」,绝不打开原文件;
//! - 代码里没有任何真实凭据(唯一的 key 常量是自造的假明文)。
//!
//! 与生产的唯一形状差异:`tauri::async_runtime::set(Handle::current())` 是
//! 进程级 OnceLock,在测试进程里 set 了会影响同进程其它测试,所以本测试不
//! set——start() 里的 serve 任务会落到 tauri 惰性建的全局 runtime 上(tauri
//! 2.11 的默认行为),语义等同,只是 runtime 归属不同。

// 每个集成测试二进制单独编译 support.rs,本探针只用 ensure_test_home,
// 其余共享助手在本二进制里是「未使用」——与 support.rs 里既有的
// #[allow(dead_code)] 同一约定,这里在模块级统一放行。
#[allow(dead_code)]
mod support;

use std::collections::HashMap;
use std::net::TcpListener;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Request as AxumRequest, State as AxumState};
use axum::http::header;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use futures::StreamExt;
use llm_usage_bar_lib::api::router::{ModelRouteInput, RouterApi, RouterProviderInput};
use llm_usage_bar_lib::route::auth::RouterUpstreamAuth;
use llm_usage_bar_lib::route::server::{self, UpstreamAuth};
use llm_usage_bar_lib::secrets::{
    BindingCredentialService, CredentialStore, CredentialStoreError, SecretString,
};
use llm_usage_bar_lib::Database;
use rusqlite::Connection as SqlConnection;

/// 探针自造的假 key 明文。不是任何真实凭据,只在本机内存与临时库里流转。
const FAKE_PROBE_KEY: &str = "sk-t21-probe-fake-key-0000000000000000000000";
/// 探针客户端故意带上的头,用来证明转发层会丢弃它们。
const CLIENT_DECOY_AUTH: &str = "Bearer client-secret-must-be-dropped";

/// 累计失败数;测试末尾断言为 0。
static FAILS: AtomicUsize = AtomicUsize::new(0);

fn check(cond: bool, name: &str, detail: String) {
    if cond {
        println!("[PASS] {name}");
    } else {
        FAILS.fetch_add(1, Ordering::Relaxed);
        println!("[FAIL] {name}: {detail}");
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64
}

/// 挑一个空闲端口:bind 0 拿系统分配的,关掉后立刻归还。
/// 与 router 的显式端口语义一致(router 永远显式指定端口)。
fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind port 0")
        .local_addr()
        .expect("local addr")
        .port()
}

/// 内存假凭据库:实现仓库公开的 CredentialStore trait,替代 OS 钥匙串。
/// 这是 auth.rs 单元测试的同一手法,只是搬到进程外跑——真实的生产路径
/// (BindingCredentialService 的预留/指纹/槽位/发布)一个都不绕。
#[derive(Default)]
struct MemoryCredentialStore {
    items: Mutex<HashMap<String, Vec<u8>>>,
}

impl CredentialStore for MemoryCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        self.items
            .lock()
            .expect("store lock")
            .insert(slot.to_string(), secret.to_vec());
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        Ok(self.items.lock().expect("store lock").get(slot).cloned())
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        self.items.lock().expect("store lock").remove(slot);
        Ok(())
    }
}

/// 假上游看到的每一次请求。auth 字段记录「上游最终收到的 Authorization」——
/// 这是证明「客户端凭据被丢弃、router 注入的凭据到达」的直接证据。
#[derive(Debug, Clone)]
struct UpstreamReqLog {
    path: String,
    authorization: Option<String>,
    model: Option<String>,
    body_len: usize,
}

#[derive(Default)]
struct UpstreamSeen {
    requests: Mutex<Vec<UpstreamReqLog>>,
}

type UpstreamState = Arc<UpstreamSeen>;

/// 假上游:一个真实的 axum 服务器,监听真实端口,返回真实的 SSE 流。
/// 它比 router 的所有单元测试假件都真:数据真的过 TCP、过 reqwest、过流。
async fn upstream_handler(AxumState(seen): AxumState<UpstreamState>, req: AxumRequest) -> Response {
    let path = req.uri().path().to_string();
    let authorization = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(req.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let model = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });

    if let Ok(mut requests) = seen.requests.lock() {
        requests.push(UpstreamReqLog {
            path,
            authorization: authorization.clone(),
            model: model.clone(),
            body_len: body.len(),
        });
    }

    // 需要凭据的模型:只有 router 注入的假 key 能过,否则 401。
    let requires_bearer = matches!(
        model.as_deref(),
        Some("fake-model-cred") | Some("fake-model-codex")
    );
    if requires_bearer
        && authorization.as_deref() != Some(format!("Bearer {FAKE_PROBE_KEY}").as_str())
    {
        return json_response(
            401,
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
        );
    }

    // 故障转移实验:这个模型永远 404(模拟「这家没有这个模型」)。
    if model.as_deref() == Some("will-404") {
        return json_response(
            404,
            r#"{"error":{"message":"The model `will-404` does not exist or you do not have access to it.","type":"invalid_request_error","code":"model_not_found"}}"#,
        );
    }

    sse_response(model.as_deref().unwrap_or("unset"))
}

fn json_response(status: u16, body: &str) -> Response {
    axum::response::Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("json response")
}

/// 一段合法的 Responses API SSE 流,末尾 response.completed 带 usage
/// (input 42 / output 17)。这是 T13 扫描器要抓的形状,也是 codex 客户端
/// 完整渲染所需的形状(output_item.done + completed 里带 output)。
fn sse_response(model: &str) -> Response {
    let events = [
        (
            "response.created",
            format!(
                r#"{{"type":"response.created","response":{{"id":"resp_t21_1","object":"response","status":"in_progress","model":"{model}","output":[]}}}}"#
            ),
        ),
        (
            "response.output_item.added",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_t21_1","type":"message","role":"assistant","status":"in_progress","content":[]}}"#.to_string(),
        ),
        (
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","item_id":"msg_t21_1","output_index":0,"content_index":0,"delta":"hello from fake upstream"}"#.to_string(),
        ),
        (
            "response.output_text.done",
            r#"{"type":"response.output_text.done","item_id":"msg_t21_1","output_index":0,"content_index":0,"text":"hello from fake upstream"}"#.to_string(),
        ),
        (
            "response.output_item.done",
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_t21_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hello from fake upstream","annotations":[]}]}}"#.to_string(),
        ),
        (
            "response.completed",
            format!(
                r#"{{"type":"response.completed","response":{{"id":"resp_t21_1","object":"response","status":"completed","model":"{model}","output":[{{"id":"msg_t21_1","type":"message","role":"assistant","status":"completed","content":[{{"type":"output_text","text":"hello from fake upstream","annotations":[]}}]}}],"usage":{{"input_tokens":42,"output_tokens":17,"total_tokens":59}}}}}}"#
            ),
        ),
    ];
    let mut sse = String::new();
    for (event, data) in events {
        sse.push_str(&format!("event: {event}\n"));
        for line in data.split('\n') {
            sse.push_str(&format!("data: {line}\n"));
        }
        sse.push('\n');
    }

    // 故意切成与事件边界无关的碎块(1..=23 循环),逼增量扫描器跨块拼事件——
    // 真实网络里 chunk 边界本来就是随机的。
    let bytes = sse.into_bytes();
    let mut chunks: Vec<Bytes> = Vec::new();
    let mut rest: &[u8] = &bytes;
    let mut size = 1usize;
    while !rest.is_empty() {
        let take = size.min(rest.len());
        chunks.push(Bytes::copy_from_slice(&rest[..take]));
        rest = &rest[take..];
        size = size % 23 + 1;
    }
    let stream = futures::stream::iter(
        chunks
            .into_iter()
            .map(Ok::<Bytes, std::convert::Infallible>),
    )
    .then(|chunk| async move {
        tokio::time::sleep(Duration::from_millis(2)).await;
        chunk
    });
    axum::response::Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from_stream(stream))
        .expect("sse response")
}

/// router_attempts 一行的原始形状(id, started_at, logical_model, provider_id,
/// outcome, failure_kind, http_status, input_tokens, output_tokens, duration_ms)。
type AttemptRow = (
    i64,
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

/// 从文件库直读 router_attempts 的全部行(任务书硬要求:查真实表,不看日志)。
fn dump_attempts(db_path: &std::path::Path) -> Vec<AttemptRow> {
    let conn = SqlConnection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open probe db read-only");
    conn.busy_timeout(Duration::from_secs(5))
        .expect("busy timeout");
    let mut statement = conn
        .prepare(
            "SELECT id, started_at, logical_model, provider_id, outcome, failure_kind,
                    http_status, input_tokens, output_tokens, duration_ms
             FROM router_attempts ORDER BY id",
        )
        .expect("prepare attempts select");
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        })
        .expect("query attempts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect attempts")
}

fn print_attempts(db_path: &std::path::Path) {
    println!("--- router_attempts 全表(SQL 直读) ---");
    let rows = dump_attempts(db_path);
    if rows.is_empty() {
        println!("(空)");
        return;
    }
    println!(
        "{:<4} {:<14} {:<22} {:<14} {:<8} {:<16} {:<6} {:<6} {:<7} duration_ms",
        "id",
        "started_at",
        "logical_model",
        "provider_id",
        "outcome",
        "failure_kind",
        "http",
        "in",
        "out",
    );
    for (id, started, logical, provider, outcome, failure, http, tin, tout, dur) in rows {
        println!(
            "{id:<4} {started:<14} {logical:<22} {provider:<14} {outcome:<8} {:<16} {:<6} {:<6} {:<7} {} ms",
            failure.as_deref().unwrap_or("-"),
            http.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            tin.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            tout.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            dur.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        );
    }
}

/// 用生产代码(未来分账面板的同一个入口)读 router_attempts 汇总。
fn print_recent_attempts(api: &RouterApi) {
    let end = now_secs() * 1000 + 60_000;
    match api.recent_attempts(0, end) {
        Ok(summaries) => {
            println!("--- RouterApi::recent_attempts(生产汇总读路径) ---");
            for summary in summaries {
                println!(
                    "provider={} attempts={} failures={} input_tokens={} output_tokens={}",
                    summary.provider_id,
                    summary.attempts,
                    summary.failures,
                    summary.input_tokens,
                    summary.output_tokens,
                );
            }
        }
        Err(error) => println!("[FAIL] recent_attempts 读路径报错: {error}"),
    }
}

/// 打 router 一发请求。故意带上一组客户端凭据头,转发层必须全部丢弃。
async fn probe(client: &reqwest::Client, router_port: u16, model: &str) -> (u16, String, String) {
    let response = client
        .post(format!("http://127.0.0.1:{router_port}/v1/responses"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .header(header::AUTHORIZATION, CLIENT_DECOY_AUTH)
        .header(header::COOKIE, "session=client-cookie-must-be-dropped")
        .header("x-api-key", "client-x-key-must-be-dropped")
        .body(
            serde_json::json!({
                "model": model,
                "input": [{"role": "user", "content": "hi"}],
                "stream": true,
            })
            .to_string(),
        )
        .send()
        .await
        .expect("probe request");
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let body = response.text().await.unwrap_or_default();
    (status, content_type, body)
}

/// 用一条 SQL 造 system provider 行与 key 行,形状与生产 reconcile /
/// create_provider_api_key 完全一致。这两个入口是 pub(crate),进程外拿不到,
/// 只能手工复刻一次(接缝,见 HANDOFF「T21 探针」一节)。
///
/// 注意:新库走 0→28 迁移链时,v17 迁移自己就会 seed system provider 目录,
/// 所以 provider 行可能已经存在——幂等处理,只保证行在那里。
fn seed_credential_rows(db_path: &std::path::Path, key_id: &str) {
    let conn = SqlConnection::open(db_path).expect("open probe db");
    conn.busy_timeout(Duration::from_secs(5))
        .expect("busy timeout");
    let now = now_secs();
    conn.execute(
        "INSERT INTO usage_providers (
             id, name, billing_kind, product_group_id, token_sources,
             quota_source, quota_interval_seconds, route_app_type, route_config,
             quota_config, enabled, needs_review, legacy_app_type,
             legacy_provider_id, created_at, updated_at, system_preset_key
         ) VALUES (
             'system-openrouter-api', 'OpenRouter', 'metered', 'openrouter-api', '[\"proxy\"]',
             NULL, NULL, 'codex', '{\"base_url\":\"https://openrouter.ai/api/v1\",\"apiFormat\":\"openai_chat\",\"authMode\":\"bearer\"}',
             NULL, 1, 0, NULL,
             NULL, ?1, ?1, 'openrouter-api'
         )
         ON CONFLICT(id) DO NOTHING",
        [now],
    )
    .expect("insert system provider row");
    let provider_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_providers WHERE id = 'system-openrouter-api')",
            [],
            |row| row.get(0),
        )
        .expect("check system provider");
    assert!(provider_exists, "system provider 行必须存在");
    conn.execute(
        "INSERT INTO provider_api_keys (
             id, provider_id, label, credential_version, sort_order, created_at, updated_at
         ) VALUES (?1, 'system-openrouter-api', 't21 probe key', 0, 0, ?2, ?2)",
        rusqlite::params![key_id, now],
    )
    .expect("insert key row");
}

/// 阶段 A:真实 start() + 真实 router_attempts(命题 1–4)。
/// 返回 (router 端口, 假上游日志) —— 日志要留到阶段 C 之后,
/// 验证真实 codex 客户端带来的凭据头同样被丢弃。
async fn phase_router(tmp: &std::path::Path) -> (u16, UpstreamState) {
    println!("\n===== 阶段 A:真实 route::server::start() + 真实 router_attempts =====");
    let upstream_port = free_port();
    let router_port = free_port();
    let db_path = tmp.join("probe-router.db");

    // 假上游:真实 axum 服务器。
    let seen: UpstreamState = Arc::new(UpstreamSeen::default());
    let upstream_app = Router::new()
        .route("/{*path}", post(upstream_handler))
        .with_state(seen.clone());
    let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", upstream_port))
        .await
        .expect("bind fake upstream");
    // 假上游保持到测试结束:阶段 C 的 codex 还要打它。测试函数返回时
    // runtime 掉落,任务自然终止。
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("serve fake upstream");
    });
    println!("[INFO] 假上游监听 127.0.0.1:{upstream_port}");

    // 真实文件库(不是内存库):router_attempts 就落在这张表里。
    let db = Arc::new(Database::init_at(&db_path).expect("init probe db"));
    println!(
        "[INFO] 探针库: {}(user_version 由 init_at 迁移到最新)",
        db_path.display()
    );

    // 凭据链:假 key 存进内存 store,走真实的 BindingCredentialService 写路径。
    let key_id = uuid::Uuid::new_v4().to_string();
    seed_credential_rows(&db_path, &key_id);
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));
    credentials
        .set_provider_api_key(&key_id, 0, SecretString::new(FAKE_PROBE_KEY.to_string()))
        .await
        .expect("set provider api key through real service");

    // 配置播种走 RouterApi——与将来设置界面同一入口。
    let api = RouterApi::new(db.clone());
    let upstream_base = format!("http://127.0.0.1:{upstream_port}/v1");
    let provider = |id: &str, priority: i64, wire_api: &str, auth_kind: &str, key: Option<&str>| {
        RouterProviderInput {
            id: id.to_string(),
            display_name: id.to_string(),
            base_url: upstream_base.clone(),
            wire_api: wire_api.to_string(),
            priority,
            enabled: true,
            auth_kind: auth_kind.to_string(),
            credential_key_id: key.map(str::to_string),
        }
    };
    api.upsert_provider(provider("fake-local", 1, "responses", "none", None))
        .expect("upsert fake-local");
    api.upsert_provider(provider(
        "bearer-local",
        2,
        "responses",
        "bearer_key",
        Some(&key_id),
    ))
    .expect("upsert bearer-local");
    api.upsert_provider(provider("credless-local", 3, "responses", "none", None))
        .expect("upsert credless-local");
    api.upsert_provider(provider("fallback-local", 10, "responses", "none", None))
        .expect("upsert fallback-local");
    api.upsert_provider(provider("chat-legacy", 0, "chat_completions", "none", None))
        .expect("upsert chat-legacy");

    let route = |logical: &str, upstream: &str| ModelRouteInput {
        logical_model: logical.to_string(),
        upstream_model: upstream.to_string(),
    };
    api.set_model_routes(
        "fake-local",
        vec![
            route("t21-probe", "fake-model-a"),
            route("t21-failover", "will-404"),
        ],
    )
    .expect("routes fake-local");
    api.set_model_routes(
        "bearer-local",
        vec![
            route("t21-probe-cred", "fake-model-cred"),
            route("t21-codex-model", "fake-model-codex"),
            route("gpt-5.1-codex-max", "fake-model-codex"),
            route("gpt-5.1-codex-mini", "fake-model-codex"),
            route("gpt-5.1", "fake-model-codex"),
            route("gpt-5-codex", "fake-model-codex"),
            route("gpt-5", "fake-model-codex"),
        ],
    )
    .expect("routes bearer-local");
    api.set_model_routes(
        "credless-local",
        vec![route("t21-no-cred", "fake-model-cred")],
    )
    .expect("routes credless-local");
    api.set_model_routes(
        "fallback-local",
        vec![route("t21-failover", "fake-model-fb")],
    )
    .expect("routes fallback-local");
    api.set_model_routes("chat-legacy", vec![route("t21-probe", "chat-only-model")])
        .expect("routes chat-legacy");

    // 真实 T9 认证实现(不写任何假 UpstreamAuth)。
    let auth: Arc<dyn UpstreamAuth> = Arc::new(RouterUpstreamAuth::new(db.clone(), credentials));

    // 命题 1 前半:start 之前端口真的没监听。
    let before = std::net::TcpStream::connect(("127.0.0.1", router_port)).is_ok();
    check(
        !before,
        "start() 之前 router 端口不可连接",
        format!("127.0.0.1:{router_port} 竟然已经能连上"),
    );

    // 走真实入口。
    server::start(db.clone(), router_port, auth.clone())
        .await
        .expect("route::server::start");
    check(
        server::listening_port() == Some(router_port),
        "start() 后 listening_port() 公布真实端口",
        format!(
            "实际 {:?},期望 Some({router_port})",
            server::listening_port()
        ),
    );
    let connected = std::net::TcpStream::connect(("127.0.0.1", router_port)).is_ok();
    check(
        connected,
        "router 端口真实可连接(TCP 握手成功)",
        format!("127.0.0.1:{router_port} 连不上"),
    );

    // 命题 1 后半 + 命题 2/3/4。
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("probe client");

    // P1:候选队列里先遇到 wire_api 不匹配的(chat-legacy,跳过),再命中 fake-local。
    let (status, ctype, body) = probe(&client, router_port, "t21-probe").await;
    check(
        status == 200,
        "P1 t21-probe 返回 200",
        format!("实际 {status}"),
    );
    check(
        ctype.starts_with("text/event-stream"),
        "P1 响应是 SSE",
        format!("实际 {ctype}"),
    );
    check(
        body.contains("fake-model-a"),
        "P1 上游看到被改写的 model(T4 改写生效)",
        "响应体不含 fake-model-a".to_string(),
    );
    check(
        body.contains("hello from fake upstream"),
        "P1 流式内容透传回客户端",
        "响应体不含上游文本".to_string(),
    );
    check(
        body.contains("\"input_tokens\":42"),
        "P1 usage 事件透传",
        "响应体不含 usage".to_string(),
    );

    // P2:凭据注入——bearer_key 走真实 T9 路径,上游必须收到注入的假 key。
    let (status, _, body) = probe(&client, router_port, "t21-probe-cred").await;
    check(
        status == 200,
        "P2 t21-probe-cred 返回 200",
        format!("实际 {status}: {body}"),
    );
    check(
        body.contains("fake-model-cred"),
        "P2 上游看到改写的 model",
        "响应体不含 fake-model-cred".to_string(),
    );

    // P3:故障转移——fake-local 404(model_not_found)→ 换 fallback-local 成功。
    let (status, _, body) = probe(&client, router_port, "t21-failover").await;
    check(
        status == 200,
        "P3 故障转移最终 200",
        format!("实际 {status}: {body}"),
    );
    check(
        body.contains("fake-model-fb"),
        "P3 第二家候选接手(队列真走下去了)",
        "响应体不含 fake-model-fb".to_string(),
    );

    // P4:拉黑生效——再打同一模型,fake-local 已被拉黑 600 秒,直接走 fallback。
    let (status, _, body) = probe(&client, router_port, "t21-failover").await;
    check(
        status == 200,
        "P4 第二次故障转移请求 200",
        format!("实际 {status}: {body}"),
    );
    check(
        body.contains("fake-model-fb"),
        "P4 命中拉黑后仍由 fallback 服务",
        "响应体不含 fake-model-fb".to_string(),
    );

    // P5:无凭据的 provider 打需要凭据的模型 → 上游 401 → 原样透传。
    let (status, _, body) = probe(&client, router_port, "t21-no-cred").await;
    check(
        status == 401,
        "P5 上游 401 原样透传(request_rejected 不换家)",
        format!("实际 {status}: {body}"),
    );
    check(
        body.contains("Incorrect API key"),
        "P5 上游错误体透传",
        format!("实际响应体: {body}"),
    );

    // P6(T12 守卫接缝):端口被占时 start 返回 Err,且不污染 listening_port。
    let occupied = TcpListener::bind(("127.0.0.1", 0)).expect("occupy port");
    let occupied_port = occupied.local_addr().expect("occupied addr").port();
    let second = server::start(db.clone(), occupied_port, auth).await;
    check(
        second.is_err(),
        "P6 端口被占时 start() 返回 Err",
        format!("实际 {second:?}"),
    );
    check(
        server::listening_port() == Some(router_port),
        "P6 失败启动不污染 listening_port",
        format!("实际 {:?}", server::listening_port()),
    );
    drop(occupied);

    // 让回填与记账落定,然后直读真实表。
    tokio::time::sleep(Duration::from_millis(300)).await;
    print_attempts(&db_path);
    print_recent_attempts(&api);

    let rows = dump_attempts(&db_path);
    let attempts_for = |provider: &str| -> Vec<&AttemptRow> {
        rows.iter().filter(|row| row.3 == provider).collect()
    };
    // P1 两行:chat-legacy skipped(协议不匹配),fake-local success。
    let p1_rows = attempts_for("fake-local");
    check(
        !p1_rows.is_empty()
            && p1_rows
                .iter()
                .any(|row| row.4 == "success" && row.6 == Some(200)),
        "P1 router_attempts: fake-local 记了 success/200",
        format!("fake-local 行: {p1_rows:?}"),
    );
    let legacy_rows = attempts_for("chat-legacy");
    check(
        legacy_rows.iter().any(|row| row.4 == "skipped"),
        "P1 router_attempts: chat-legacy 记了 skipped(wire_api 不匹配)",
        format!("chat-legacy 行: {legacy_rows:?}"),
    );
    // 命题 4:token 回填(42/17 来自真实 SSE 流的 response.completed)。
    let success_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.4 == "success" && (row.3 == "fake-local" || row.3 == "bearer-local"))
        .cloned()
        .collect();
    check(
        !success_rows.is_empty()
            && success_rows
                .iter()
                .all(|row| row.7 == Some(42) && row.8 == Some(17)),
        "命题 4: 成功行的 input_tokens/output_tokens 被 T13 回填为 42/17",
        format!("成功行: {success_rows:?}"),
    );
    let failover_rows = attempts_for("fake-local");
    check(
        failover_rows.iter().any(|row| {
            row.4 == "failed" && row.5.as_deref() == Some("model_not_found") && row.6 == Some(404)
        }),
        "P3 router_attempts: fake-local 记了 failed/model_not_found/404",
        format!("fake-local 行: {failover_rows:?}"),
    );
    check(
        attempts_for("fallback-local")
            .iter()
            .any(|row| row.4 == "success"),
        "P3 router_attempts: fallback-local 记了 success",
        format!("fallback-local 行: {:?}", attempts_for("fallback-local")),
    );
    let bearer_rows = attempts_for("bearer-local");
    check(
        bearer_rows.iter().any(|row| row.4 == "success"),
        "P2 router_attempts: bearer-local 记了 success",
        format!("bearer-local 行: {bearer_rows:?}"),
    );
    check(
        attempts_for("credless-local").iter().any(|row| {
            row.4 == "failed" && row.5.as_deref() == Some("request_rejected") && row.6 == Some(401)
        }),
        "P5 router_attempts: credless-local 记了 failed/request_rejected/401",
        format!("credless-local 行: {:?}", attempts_for("credless-local")),
    );

    // 安全面:客户端带上的凭据头一个都没漏到上游。
    let upstream_log = seen.requests.lock().expect("upstream log").clone();
    check(
        upstream_log
            .iter()
            .all(|entry| entry.authorization.as_deref() != Some(CLIENT_DECOY_AUTH)),
        "安全面: 客户端 authorization 从未出现在上游",
        format!("上游日志: {upstream_log:?}"),
    );
    let bearer_seen = upstream_log.iter().any(|entry| {
        entry.model.as_deref() == Some("fake-model-cred")
            && entry.authorization.as_deref() == Some(format!("Bearer {FAKE_PROBE_KEY}").as_str())
    });
    check(
        bearer_seen,
        "P2 上游收到了 router 注入的凭据头(真实 T9 链路)",
        format!("上游日志: {upstream_log:?}"),
    );
    let paths: Vec<&str> = upstream_log
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    check(
        paths.iter().all(|path| *path == "/v1/responses"),
        "上游收到的路径全部是 /v1/responses(跨任务硬契约)",
        format!("路径集合: {paths:?}"),
    );

    println!("[INFO] 假上游收到的全部请求:");
    for entry in &upstream_log {
        println!(
            "  path={} auth={} model={:?} body_len={}",
            entry.path,
            entry.authorization.as_deref().unwrap_or("-"),
            entry.model,
            entry.body_len,
        );
    }

    (router_port, seen)
}

/// 阶段 B:旧版库副本跑迁移(命题 5)。默认跳过;经 `T21_PROBE_MIGRATE_FROM`
/// 指定「用户自己复制出来的副本」后执行。绝不打开环境变量指向的原文件。
fn phase_migration(tmp: &std::path::Path) {
    println!("\n===== 阶段 B:旧版库副本迁移(命题 5) =====");
    let Ok(source) = std::env::var("T21_PROBE_MIGRATE_FROM") else {
        println!("[SKIP] 未设置 T21_PROBE_MIGRATE_FROM;迁移实验需要你提供旧版库的副本路径");
        return;
    };
    let source = std::path::PathBuf::from(source);
    let copy = tmp.join("migrate-copy.db");
    std::fs::copy(&source, &copy).expect("copy migration source to temp");
    println!("[INFO] 已复制 {source:?} 到副本: {}", copy.display());

    // 迁移前的快照:版本 + 每张用户表的行数。
    let snapshot = |path: &std::path::Path| -> (i64, Vec<(String, i64)>) {
        let conn = SqlConnection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open snapshot");
        conn.busy_timeout(Duration::from_secs(5))
            .expect("busy timeout");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user version");
        let mut tables = Vec::new();
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .expect("list tables");
        let names: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .expect("query tables")
            .collect::<Result<_, _>>()
            .expect("collect tables");
        drop(statement);
        for name in names {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |row| {
                    row.get(0)
                })
                .expect("count table");
            tables.push((name, count));
        }
        (version, tables)
    };

    let (before_version, before) = snapshot(&copy);
    println!(
        "[INFO] 迁移前 user_version = {before_version},{} 张表",
        before.len()
    );

    // 在副本上真实打开:走 init_at 的完整迁移链。
    let db = Database::init_at(&copy).expect("open migration copy through real init path");
    check(
        db.database_path() == Some(copy.as_path()),
        "Database::init_at 打开的是副本而不是原文件",
        format!("实际路径 {:?}", db.database_path()),
    );
    drop(db);

    let (after_version, after) = snapshot(&copy);
    check(
        after_version == 28,
        "副本迁移后 user_version = 28",
        format!("实际 {after_version}"),
    );
    let router_tables: Vec<&str> = after
        .iter()
        .filter(|(name, _)| name.starts_with("router_"))
        .map(|(name, _)| name.as_str())
        .collect();
    check(
        router_tables.contains(&"router_providers")
            && router_tables.contains(&"router_model_map")
            && router_tables.contains(&"router_attempts"),
        "迁移后三张 router 表存在",
        format!("实际: {router_tables:?}"),
    );
    let attempts_count = after
        .iter()
        .find(|(name, _)| name == "router_attempts")
        .map(|(_, count)| *count)
        .unwrap_or(-1);
    check(
        attempts_count == 0,
        "迁移后 router_attempts 为空(旧数据没被塞进路由表)",
        format!("实际 {attempts_count} 行"),
    );

    // 逐表比对:除了「预期会变」的表(prune/seed),其余行数必须原样。
    let before_map: HashMap<&str, i64> = before.iter().map(|(n, c)| (n.as_str(), *c)).collect();
    println!("--- 逐表行数 diff(before -> after) ---");
    let mut unexpected: Vec<String> = Vec::new();
    for (name, after_count) in &after {
        let before_count = before_map.get(name.as_str()).copied().unwrap_or(0);
        if before_count != *after_count {
            println!("  {name}: {before_count} -> {after_count}");
            // 新表、定价种子表、会被启动清理裁剪的表是预期变化,其余都要盯。
            let expected_change = name.starts_with("router_")
                || name == "model_pricing"
                || name.contains("session_log")
                || name.contains("rollup")
                || name == "proxy_request_logs";
            if !expected_change {
                unexpected.push(format!("{name}: {before_count} -> {after_count}"));
            }
        }
    }
    check(
        unexpected.is_empty(),
        "迁移没有让非预期表发生行数变化",
        format!("非预期变化: {unexpected:?}"),
    );
    // 核心业务表抽查:settings 一行都不能少。
    let settings_unchanged = before_map.get("settings")
        == after
            .iter()
            .find(|(name, _)| name == "settings")
            .map(|(_, count)| count);
    check(
        settings_unchanged,
        "settings 表行数原样保留",
        format!(
            "before={:?} after={:?}",
            before_map.get("settings"),
            after
                .iter()
                .find(|(name, _)| name == "settings")
                .map(|(_, c)| c)
        ),
    );
}

/// 阶段 C:真实 Codex CLI 打进 router(命题 6,受红线 1 约束)。
/// 不动真实 ~/.codex,用独立临时 CODEX_HOME,里面放一份与指针写入同形状的配置。
async fn phase_codex(tmp: &std::path::Path, router_port: u16) {
    println!("\n===== 阶段 C:真实 Codex CLI 打进来(命题 6) =====");
    let codex_home = tmp.join("codex-home");
    let project = tmp.join("codex-project");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::create_dir_all(&project).expect("create codex project");

    // 与 pointer.rs 写出的配置同形状:model_provider + [model_providers.llm_usage_bar_router]。
    // 多一个 env_key 用于「客户端凭据被 router 丢弃」的验证。
    let config = format!(
        "model_provider = \"llm_usage_bar_router\"\n\n\
         [model_providers.llm_usage_bar_router]\n\
         name = \"T21 Probe Router\"\n\
         base_url = \"http://127.0.0.1:{router_port}/v1\"\n\
         wire_api = \"responses\"\n\
         env_key = \"T21_PROBE_TOKEN\"\n"
    );
    std::fs::write(codex_home.join("config.toml"), config).expect("write temp codex config");
    println!(
        "[INFO] 临时 CODEX_HOME: {}(真实 ~/.codex 全程未碰)",
        codex_home.display()
    );

    // 用 std::process 跑 codex,避免引入 tokio process feature;外面套超时。
    // -m 指定一个只在探针库里存在的逻辑模型:codex 发什么 model 完全由我们
    // 决定,改写与路由的每个字节都可预期。
    let run = StdCommand::new("codex")
        .args([
            "exec",
            "-m",
            "t21-codex-model",
            "-s",
            "read-only",
            "--ephemeral",
            "--skip-git-repo-check",
            "Reply with exactly this text: hello from fake upstream",
        ])
        .env("CODEX_HOME", &codex_home)
        .env("T21_PROBE_TOKEN", CLIENT_DECOY_AUTH)
        .env("CODEX_DISABLE_TELEMETRY", "1")
        .current_dir(&project)
        .output();
    let result = tokio::time::timeout(
        Duration::from_secs(240),
        tokio::task::spawn_blocking(move || run),
    )
    .await
    .expect("codex timeout wrapper")
    .expect("spawn_blocking join");
    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("[INFO] codex 退出码: {:?}", output.status.code());
            println!("--- codex stdout ---\n{stdout}");
            println!("--- codex stderr ---\n{stderr}");
            check(
                output.status.success(),
                "codex exec 正常退出",
                format!(
                    "stderr 尾部: {}",
                    &stderr[stderr.len().saturating_sub(500)..]
                ),
            );
            check(
                stdout.contains("hello from fake upstream"),
                "codex 收到了 router 转发的假上游输出(全链路通)",
                "stdout 不含目标文本".to_string(),
            );
        }
        Err(error) => {
            // 机器上没装 codex 不是 router 的失败:跳过而不是记 FAIL。
            println!("[SKIP] codex 二进制不可用,阶段 C 跳过: {error}");
        }
    }
}

/// 阶段 D:真实上游(红线 3)。默认跳过;用户明确同意后经环境变量开启,
/// 发**一个**最小请求。token 只从环境变量读,不打印、不写盘。
async fn phase_real_upstream(tmp: &std::path::Path) {
    println!("\n===== 阶段 D:真实上游(默认关闭) =====");
    let Ok(token) = std::env::var("T21_PROBE_REAL_TOKEN") else {
        println!("[SKIP] 未设置 T21_PROBE_REAL_TOKEN;真实上游实验默认关闭(会真花钱)");
        return;
    };
    let base_url = std::env::var("T21_PROBE_REAL_BASE_URL")
        .unwrap_or_else(|_| "https://www.packyapi.ai/v1".to_string());
    println!(
        "[INFO] base_url={} token 长度={}(内容不打印)",
        base_url,
        token.len()
    );

    // 独立的小环境:自己的库、自己的 router 实例,与假上游阶段互不干扰。
    let db_path = tmp.join("probe-real.db");
    let db = Arc::new(Database::init_at(&db_path).expect("init real db"));
    let key_id = uuid::Uuid::new_v4().to_string();
    seed_credential_rows(&db_path, &key_id);
    let store = Arc::new(MemoryCredentialStore::default());
    let credentials = Arc::new(BindingCredentialService::new(db.clone(), store));
    // token 的两份副本:一份进凭据服务(router 注入用),一份留给 /models
    // 探测(免费入口,进程内拼好即用即弃)。两者都在本进程内,不打印不落盘。
    let models_token = token.clone();
    credentials
        .set_provider_api_key(&key_id, 0, SecretString::new(token))
        .await
        .expect("seed real token through real service");

    let api = RouterApi::new(db.clone());
    api.upsert_provider(RouterProviderInput {
        id: "packyapi-real".to_string(),
        display_name: "packyapi-real".to_string(),
        base_url: base_url.clone(),
        wire_api: "responses".to_string(),
        priority: 1,
        enabled: true,
        auth_kind: "bearer_key".to_string(),
        credential_key_id: Some(key_id),
    })
    .expect("upsert real provider");

    // 先用免费入口 /models 看这家到底有哪些模型,再挑一个打。
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("real client");
    let models_response = client
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .header(header::AUTHORIZATION, format!("Bearer {models_token}"))
        .send()
        .await;
    let mut chosen_model: Option<String> = std::env::var("T21_PROBE_REAL_MODEL").ok();
    match models_response {
        Ok(response) if response.status().is_success() => {
            let body = response.text().await.unwrap_or_default();
            let ids: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("data").cloned())
                .and_then(|data| serde_json::from_value::<Vec<serde_json::Value>>(data).ok())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            println!("[INFO] /models 返回 {} 个模型(免费入口)", ids.len());
            println!("[INFO] 前 10 个: {:?}", &ids[..ids.len().min(10)]);
            if chosen_model.is_none() {
                chosen_model = ids
                    .iter()
                    .find(|id| id.contains("codex"))
                    .or_else(|| ids.iter().find(|id| id.contains("gpt-5")))
                    .or_else(|| ids.first())
                    .cloned();
            }
        }
        Ok(response) => {
            println!("[INFO] /models 返回 {}", response.status());
        }
        Err(error) => {
            println!("[INFO] /models 请求失败: {error}");
        }
    }
    let chosen_model = chosen_model.unwrap_or_else(|| "gpt-5.6-sol".to_string());
    println!("[INFO] 选用模型: {chosen_model}");

    api.set_model_routes(
        "packyapi-real",
        vec![ModelRouteInput {
            logical_model: "t21-real-probe".to_string(),
            upstream_model: chosen_model.clone(),
        }],
    )
    .expect("route real model");

    let router_port = free_port();
    let auth: Arc<dyn UpstreamAuth> = Arc::new(RouterUpstreamAuth::new(db.clone(), credentials));
    server::start(db.clone(), router_port, auth)
        .await
        .expect("start real router");

    // 最小请求:一个词,流式。
    let (status, ctype, body) = probe(&client, router_port, "t21-real-probe").await;
    check(
        status == 200,
        "真实上游请求返回 200",
        format!("实际 {status}: {body}"),
    );
    check(
        ctype.starts_with("text/event-stream"),
        "真实上游响应是 SSE",
        format!("实际 {ctype}"),
    );
    println!("--- 真实上游响应体(前 1200 字节) ---");
    println!("{}", &body[..body.len().min(1200)]);

    tokio::time::sleep(Duration::from_millis(500)).await;
    let rows = dump_attempts(&db_path);
    for row in rows.iter().filter(|row| row.2 == "t21-real-probe") {
        println!(
            "真实上游 router_attempts 行: provider={} outcome={} failure={:?} http={:?} input_tokens={:?} output_tokens={:?} duration_ms={:?}",
            row.3, row.4, row.5, row.6, row.7, row.8, row.9,
        );
    }
    check(
        rows.iter().any(|row| {
            row.2 == "t21-real-probe" && row.4 == "success" && row.7.is_some() && row.8.is_some()
        }),
        "真实上游的 router_attempts 记了 success 且 token 回填(T13 对真实 SSE 生效)",
        format!(
            "t21-real-probe 行: {:?}",
            rows.iter()
                .filter(|r| r.2 == "t21-real-probe")
                .collect::<Vec<_>>()
        ),
    );
}

/// T21 端到端探针主测试。#[ignore]:平时 cargo test 不跑,
/// 只在 `cargo test -- --ignored` 时显式跑(现仓库 ignored 测试 2 → 3)。
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn router_end_to_end_probe() {
    // 红线 1/2 的结构性保证:一切 home 解析都先落到隔离目录。
    support::ensure_test_home();

    let tmp = std::env::temp_dir().join(format!("t21-router-probe-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    println!(
        "[INFO] 隔离 home: {}",
        support::ensure_test_home().display()
    );
    println!("[INFO] 临时目录: {}", tmp.display());

    let (router_port, seen) = phase_router(&tmp).await;
    phase_migration(&tmp);
    phase_codex(&tmp, router_port).await;
    phase_real_upstream(&tmp).await;

    // codex 阶段后重读 attempts,补记 codex 那一行。
    let db_path = tmp.join("probe-router.db");
    println!("\n--- codex 阶段后的 router_attempts ---");
    print_attempts(&db_path);
    let rows = dump_attempts(&db_path);
    let codex_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.4 == "success" && row.3 == "bearer-local")
        .collect();
    check(
        codex_rows
            .iter()
            .any(|row| row.7 == Some(42) && row.8 == Some(17)),
        "codex 请求的 router_attempts 行存在且 token 回填 42/17",
        format!("bearer-local success 行: {codex_rows:?}"),
    );
    // 真实 codex 客户端打进来的那一次,上游看到的仍只有 router 注入的凭据:
    // codex 自带的客户端 token 在转发层被丢弃(安全面,对真实客户端同样成立)。
    let after_codex_log = seen.requests.lock().expect("upstream log").clone();
    check(
        after_codex_log
            .iter()
            .all(|entry| entry.authorization.as_deref() != Some(CLIENT_DECOY_AUTH)),
        "codex 客户端凭据头同样被丢弃(上游从未看到)",
        format!("上游日志: {after_codex_log:?}"),
    );
    let codex_upstream_hit = after_codex_log.iter().any(|entry| {
        entry.model.as_deref() == Some("fake-model-codex")
            && entry.authorization.as_deref() == Some(format!("Bearer {FAKE_PROBE_KEY}").as_str())
    });
    check(
        codex_upstream_hit,
        "codex 请求到达上游时带着 router 注入的凭据",
        format!("上游日志: {after_codex_log:?}"),
    );
    let codex_path_ok = after_codex_log.iter().any(|entry| {
        entry.model.as_deref() == Some("fake-model-codex") && entry.path == "/v1/responses"
    });
    check(
        codex_path_ok,
        "真实 codex 打进来的路径是 /v1/responses(base_url 含 /v1 的跨任务契约成立)",
        format!("上游日志: {after_codex_log:?}"),
    );
    println!("[INFO] codex 阶段后假上游新增请求:");
    for entry in &after_codex_log {
        if entry.model.as_deref() == Some("fake-model-codex") {
            println!(
                "  path={} auth={} model={:?} body_len={}",
                entry.path,
                entry.authorization.as_deref().unwrap_or("-"),
                entry.model,
                entry.body_len,
            );
        }
    }

    let fails = FAILS.load(Ordering::Relaxed);
    println!("\n===== 汇总:{fails} 项失败 =====");
    println!("[INFO] 临时目录保留供复查: {}", tmp.display());
    assert_eq!(fails, 0, "存在失败的探针断言,见上方 [FAIL] 输出");
}
