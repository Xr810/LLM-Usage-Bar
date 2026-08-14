//! HTTP 转发层:loopback 上的本地路由服务(T6)。
//!
//! 任务书:docs/tasks/T6-router-forward.md。Codex 打到 `POST /v1/responses`
//! (base_url 含 `/v1` 是跨任务硬契约),这里按 T4 的候选队列依次转发,失败用
//! T5 的 `classify` 判定换不换家,每次尝试记进 T3 的 `router_attempts`。
//!
//! 安全面(§4.3,本任务唯一的网络出口):转给上游的头只有白名单
//! `content-type`、`accept`,加上 `UpstreamAuth` 注入的凭据头。客户端来的
//! `authorization` / `cookie` / `x-api-key` / `openai-organization` 一律丢弃。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::{Body, BodyDataStream, Bytes};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use futures::future::BoxFuture;
use futures::stream::StreamExt;

use crate::database::{AttemptOutcome, Database, RouterAttempt, RouterProvider, WireApi};
use crate::error::AppError;
use crate::router::decision::{candidates_for, Blacklisted, Candidate, RouteMode};
use crate::router::failure::{classify, AttemptResult, BlacklistScope, FailureKind};

/// 请求体缓存上限(§4.2)。超过之后不缓存、退化成纯流式,不再有故障转移能力。
const MAX_BUFFERED_BODY: usize = 4 * 1024 * 1024;
/// 上游总超时:覆盖共享 client 的 600 秒(§1.1 坑一),长 SSE 流不能被半路砍断。
const UPSTREAM_TOTAL_TIMEOUT_SECS: u64 = 3600;
/// 首字节超时:共享 client 没有 read_timeout,这个信号要自己造(§1.1 坑二),
/// 只包住「发出去到拿到响应头」这一段,首字节之后的流不再套超时。
const FIRST_BYTE_TIMEOUT_SECS: u64 = 60;
/// 错误体给 classify 做「模型不存在」判断的片段上限,只需关键词不用全文。
const ERROR_SNIPPET_MAX: usize = 8 * 1024;
/// 排干(丢弃)一个不会再转发的错误体时最多读多少字节:恶意上游可以无限发
/// 错误体,不能陪它读完,读一点让连接有机会回池即可。
const ERROR_DRAIN_MAX: usize = 1024 * 1024;

/// 上游认证头的提供方。**router 自己不持有任何凭据**——它只在要发请求的那一刻
/// 问一次「这家的认证头是什么」，拿到就用，用完不留。
///
/// 返回的是要原样加到上游请求上的头（通常是一个 `Authorization`，
/// 但某些 provider 需要多个，所以是 Vec）。
/// 返回 `Ok(vec![])` 表示这家不需要认证头；返回 `Err` 表示凭据取不到，
/// 调用方应把这个候选当作失败（记 `auth_unavailable`），并继续下一个候选——
/// 换一家是另一份凭据，值得试。
/// 返回 future 而不是直接返回值:取凭据要走 `CredentialService`,那条路径是
/// async(底层 `spawn_blocking` 读钥匙串)。同步签名会逼实现方 `block_on`,
/// 而 router 就跑在 tokio worker 上——那是死锁。形状照抄仓库既有的
/// `QuotaCollector`(`usage/quota.rs:86`)。
pub trait UpstreamAuth: Send + Sync {
    fn headers_for<'a>(
        &'a self,
        provider_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, AppError>>;
}

/// 已经成功绑定的端口。`start()` 绑定成功后写入，失败时保持 None。
///
/// 用 AtomicU16 而不是 Mutex：这是一个只写一次、之后频繁读的标量，
/// 而且读的一方（tauri 命令）不该有任何机会阻塞在 router 的锁上。
static LISTENING_PORT: AtomicU16 = AtomicU16::new(0);

/// router 当前监听的端口；`None` 表示没起来。
///
/// 0 是「未绑定」的哨兵值——端口 0 在 bind 语义里是「随便给一个」，
/// 而我们永远显式指定端口，所以它不会是一个真实的监听端口。
pub fn listening_port() -> Option<u16> {
    match LISTENING_PORT.load(Ordering::Relaxed) {
        0 => None,
        port => Some(port),
    }
}

/// 启动本地路由服务。绑定失败返回 Err，由调用方决定怎么提示。
///
/// 必须在 app 启动流程的**早期**调用——先把端口开起来，再做其余初始化
/// (设计文档决定 5②)。
///
/// `auth` 由启动方注入。**本任务不实现它**，只定义 trait 并在转发时调用；
/// 测试里用一个返回固定头的假实现。真实实现由 provider 侧在别的任务里补。
pub async fn start(
    db: Arc<Database>,
    port: u16,
    auth: Arc<dyn UpstreamAuth>,
) -> Result<(), AppError> {
    let state = Arc::new(RouterState {
        db,
        auth,
        sender: Arc::new(ReqwestSender),
        blacklist: Mutex::new(Vec::new()),
    });
    let listener = bind_loopback(port).await?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::Config(format!("读取监听地址失败: {error}")))?;
    // 绑定成功之后、serve 之前公布端口：守卫读到的是「已经绑定」的事实，
    // 不是「即将绑定」的意图。绑定失败走上面的 `?` 直接返回，不会写。
    LISTENING_PORT.store(port, Ordering::Relaxed);
    let app = build_router(state);
    tauri::async_runtime::spawn(async move {
        log::info!("本地 router 已启动: http://{address}/v1/responses");
        if let Err(error) = axum::serve(listener, app).await {
            log::error!("本地 router 服务退出: {error}");
        }
        // serve 返回（正常退出或出错）之后服务已不在监听：清掉公布值，
        // 否则 listening_port() 会继续谎报「router 活着」，守卫形同虚设。
        LISTENING_PORT.store(0, Ordering::Relaxed);
    });
    Ok(())
}

/// 绑定 127.0.0.1:port。监听地址固定 loopback,绝不 0.0.0.0(§3)——这是
/// 本机服务,暴露到网络上是安全问题。
async fn bind_loopback(port: u16) -> Result<tokio::net::TcpListener, AppError> {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| AppError::Config(format!("绑定 127.0.0.1:{port} 失败: {error}")))
}

/// 路由表:只注册 `POST /v1/responses` 一条(§3.1),其余路径一律 404。
/// `/v1` 前缀不是可选的——T7 把 Codex 的 base_url 写成
/// `http://127.0.0.1:<port>/v1`,Codex 再往后面接 `/responses`。
fn build_router(state: Arc<RouterState>) -> Router {
    Router::new()
        .route("/v1/responses", post(handle_responses))
        .fallback(not_found)
        .with_state(state)
}

/// 其余路径一律 404(§3.1)。fallback 处理器必须显式带上状态类型,
/// axum 0.8 不会给无状态闭包补 Handler<T, S> 的实现。
async fn not_found(_state: State<Arc<RouterState>>) -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

/// 路由服务的共享状态。拉黑列表放这里而不是全局 OnceLock:每个服务实例一份,
/// 测试互不串扰(任务书 §5 说「用 OnceLock 或等价物」,这就是等价物)。
struct RouterState {
    db: Arc<Database>,
    auth: Arc<dyn UpstreamAuth>,
    sender: Arc<dyn UpstreamSender>,
    /// 拉黑状态(内存,不落库)。过期靠读的时候判 `until_ms > now`,
    /// 不起任何后台清理任务(§6.4 第 2 条)。
    blacklist: Mutex<Vec<Blacklisted>>,
}

/// 一次出站请求。body 分两种:完整缓冲(Bytes,可重发)与超限流(只能发给
/// 一个候选,发完即失效)。
struct OutboundRequest {
    url: String,
    headers: Vec<(String, String)>,
    body: OutboundBody,
}

enum OutboundBody {
    /// ≤ 4 MB 的完整请求体,可以原样重发给下一家。
    Buffered(Bytes),
    /// > 4 MB:改写后的前缀 + 剩余流,只能转发一次(§4.2)。
    Stream { head: Bytes, tail: BodyDataStream },
}

/// 发送失败的信号。生产实现里从 reqwest 的结果翻译而来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendFailure {
    /// 连不上、DNS 失败、TLS 失败。
    Connect,
    /// 60 秒内没有第一个字节。
    FirstByteTimeout,
}

/// 「发一次上游请求」的抽象:生产实现走共享 reqwest client,
/// 测试换成脚本化假实现(任务书 §8:不要为了测试起真实上游服务)。
trait UpstreamSender: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> Pin<Box<dyn Future<Output = Result<reqwest::Response, SendFailure>> + Send>>;
}

/// 生产实现:共享 client + 两个超时(§1.1)。
struct ReqwestSender;

impl UpstreamSender for ReqwestSender {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> Pin<Box<dyn Future<Output = Result<reqwest::Response, SendFailure>> + Send>> {
        Box::pin(send_with_reqwest(request))
    }
}

async fn send_with_reqwest(request: OutboundRequest) -> Result<reqwest::Response, SendFailure> {
    let client = crate::http_client::get();
    let mut builder = client
        .post(&request.url)
        // 坑一:覆盖共享 client 的 600 秒总超时,长 SSE 流跑过 10 分钟会被拦腰砍断。
        .timeout(Duration::from_secs(UPSTREAM_TOTAL_TIMEOUT_SECS));
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    builder = match request.body {
        OutboundBody::Buffered(bytes) => builder.body(bytes),
        OutboundBody::Stream { head, tail } => builder.body(reqwest::Body::wrap_stream(
            futures::stream::once(async move { Ok::<Bytes, axum::Error>(head) }).chain(tail),
        )),
    };
    // 坑二:首字节超时要自己造——只包住「发出去到拿到响应头」这一段。
    match tokio::time::timeout(Duration::from_secs(FIRST_BYTE_TIMEOUT_SECS), builder.send()).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => Err(SendFailure::Connect),
        Err(_) => Err(SendFailure::FirstByteTimeout),
    }
}

/// 读进来的请求体。≤ 4 MB 完整缓存(要能重发给下一家);超过上限只缓存前缀
/// (还要从里面取 model),溢出的部分和剩余流原样转发。
enum InboundBody {
    Buffered(Bytes),
    Oversize { head: Bytes, tail: BodyDataStream },
}

async fn read_request_body(body: Body) -> Result<InboundBody, AppError> {
    let mut stream = body.into_data_stream();
    let mut head: Vec<u8> = Vec::with_capacity(MAX_BUFFERED_BODY.min(64 * 1024));
    loop {
        let Some(chunk) = stream.next().await else {
            return Ok(InboundBody::Buffered(Bytes::from(head)));
        };
        let chunk =
            chunk.map_err(|error| AppError::InvalidInput(format!("读取请求体失败: {error}")))?;
        if head.len() + chunk.len() > MAX_BUFFERED_BODY {
            let room = MAX_BUFFERED_BODY - head.len();
            let remainder = if room == 0 {
                chunk
            } else {
                head.extend_from_slice(&chunk[..room]);
                chunk.slice(room..)
            };
            let tail = Body::from_stream(
                futures::stream::once(async move { Ok::<Bytes, axum::Error>(remainder) })
                    .chain(stream),
            );
            return Ok(InboundBody::Oversize {
                head: Bytes::from(head),
                tail: tail.into_data_stream(),
            });
        }
        head.extend_from_slice(&chunk);
    }
}

/// 取逻辑模型:完整缓存体走整体 JSON 解析取顶层 model;超限体只能扫已缓存
/// 前缀(Codex 生成的请求体里顶层 model 永远在最前,见前缀扫描函数的说明)。
fn extract_logical_model(inbound: &InboundBody) -> Result<String, AppError> {
    match inbound {
        InboundBody::Buffered(bytes) => {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| AppError::InvalidInput(format!("请求体不是合法 JSON: {error}")))?;
            value
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| AppError::InvalidInput("请求体缺少 model 字段".to_string()))
        }
        InboundBody::Oversize { head, .. } => extract_model_from_prefix(head).ok_or_else(|| {
            AppError::InvalidInput("请求体超过缓存上限且 model 字段不在已缓存前缀内".to_string())
        }),
    }
}

/// 改写完整缓存体里的 model(§4 步骤 7a):整棵树解析、只替换顶层 model、
/// 重新序列化,其余字段一个不动(serde_json 开着 preserve_order)。
fn rewrite_buffered_body(bytes: &[u8], upstream_model: &str) -> Result<Bytes, AppError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| AppError::InvalidInput(format!("请求体不是合法 JSON: {error}")))?;
    let model_slot = value
        .get_mut("model")
        .ok_or_else(|| AppError::InvalidInput("请求体缺少 model 字段".to_string()))?;
    *model_slot = serde_json::Value::String(upstream_model.to_string());
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|error| AppError::JsonSerialize { source: error })
}

/// 在「超限请求体」的已缓存前缀里定位 `"model"` 字符串值,返回
/// (解码后的值, 值内容的起始偏移, 值内容的结束偏移,不含引号)。
///
/// 只在超限路径使用:超限体无法整体解析。JSON 字符串内部的引号必然被转义,
/// 因此裸的 `"model"` 字节序列只可能是对象键;Codex 生成的请求体里顶层
/// model 永远出现在最前面,所以取第一个命中。若 model 值恰好跨在缓存边界上
/// 被截断,返回 None,由调用方报 400——不做半截猜测。
fn locate_model_string_in_prefix(head: &[u8]) -> Option<(String, usize, usize)> {
    let key = b"\"model\"";
    let mut search_from = 0usize;
    while search_from + key.len() <= head.len() {
        let found = head[search_from..]
            .windows(key.len())
            .position(|window| window == key)?;
        let after_key = search_from + found + key.len();
        // 键之后跳过空白,必须是 ':'。
        let mut cursor = after_key;
        while cursor < head.len() && head[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= head.len() || head[cursor] != b':' {
            search_from = after_key;
            continue;
        }
        cursor += 1;
        while cursor < head.len() && head[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= head.len() || head[cursor] != b'"' {
            search_from = after_key;
            continue;
        }
        // 扫描 JSON 字符串:反斜杠转义下一个字节,引号结束。
        let value_start = cursor + 1;
        cursor += 1;
        let mut value_end = None;
        while cursor < head.len() {
            match head[cursor] {
                b'\\' => cursor += 2,
                b'"' => {
                    value_end = Some(cursor);
                    break;
                }
                _ => cursor += 1,
            }
        }
        let value_end = value_end?;
        // 用 serde_json 解出真实值(顺带校验转义)——必须把两边的引号带上,
        // 裸内容不是合法 JSON 字符串。解不动就当作没找到。
        let value: String = serde_json::from_slice(&head[value_start - 1..value_end + 1]).ok()?;
        return Some((value, value_start, value_end));
    }
    None
}

fn extract_model_from_prefix(head: &[u8]) -> Option<String> {
    locate_model_string_in_prefix(head).map(|(value, _, _)| value)
}

/// 超限体改 model:找到值在字节里的位置,把新值的 JSON 字面量原位拼进去,
/// 其余字节一个不动——不整体解析(超限体解析不动),所以只能做字节级拼接。
fn rewrite_model_in_prefix(head: &[u8], upstream_model: &str) -> Result<Bytes, AppError> {
    let (_, value_start, value_end) = locate_model_string_in_prefix(head).ok_or_else(|| {
        AppError::InvalidInput("请求体超过缓存上限且 model 字段不在已缓存前缀内".to_string())
    })?;
    let replacement = serde_json::to_string(upstream_model)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    // 替换范围要连两边的引号一起(值内容之外的开引号与闭引号),
    // 新值本身是带引号的 JSON 字面量。
    let mut out = Vec::with_capacity(head.len() + replacement.len());
    out.extend_from_slice(&head[..value_start - 1]);
    out.extend_from_slice(replacement.as_bytes());
    out.extend_from_slice(&head[value_end + 1..]);
    Ok(Bytes::from(out))
}

/// 读 router.mode(§4.4):读不到、空串、格式不认识一律 Auto,不报错、不写回
/// 默认值。理由:模式是用户的方向盘,拿不准的时候「自动」是唯一不会让他卡住
/// 的选择。
fn read_router_mode(db: &Database) -> Result<RouteMode, AppError> {
    let raw = db.get_setting("router.mode")?;
    Ok(parse_router_mode(raw.as_deref()))
}

fn parse_router_mode(raw: Option<&str>) -> RouteMode {
    match raw {
        Some("auto") | None => RouteMode::Auto,
        Some(manual) => match manual.strip_prefix("manual:") {
            Some(provider_id) if !provider_id.is_empty() => RouteMode::Manual {
                provider_id: provider_id.to_string(),
            },
            // "manual:"(空 provider)、"whatever" 等一律当 auto。
            _ => RouteMode::Auto,
        },
    }
}

/// 转给上游的头(§4.3,唯一的安全面):
/// - 白名单只有 content-type 与 accept,其余客户端头一律丢弃,所以客户端来的
///   authorization / cookie / x-api-key / openai-organization 绝不会漏给上游;
/// - UpstreamAuth 注入的凭据头原样附加。
fn build_upstream_headers(
    client_headers: &HeaderMap,
    auth_headers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for name in [header::CONTENT_TYPE, header::ACCEPT] {
        if let Some(value) = client_headers.get(&name) {
            if let Ok(value) = value.to_str() {
                out.push((name.to_string(), value.to_string()));
            }
        }
    }
    out.extend_from_slice(auth_headers);
    out
}

/// 写拉黑(§5):内存、不落库、不起清理任务。同一 (provider, 逻辑模型) 只保留
/// 最新一条,列表长度上界是配置里的路由行数,不会无限增长。
fn push_blacklist(
    blacklist: &Mutex<Vec<Blacklisted>>,
    provider_id: &str,
    scope: BlacklistScope,
    logical_model: &str,
    now_ms: i64,
    cooldown_secs: i64,
) {
    let entry = Blacklisted {
        provider_id: provider_id.to_string(),
        logical_model: match scope {
            BlacklistScope::ThisRoute => Some(logical_model.to_string()),
            BlacklistScope::WholeProvider => None,
        },
        until_ms: now_ms + cooldown_secs * 1000,
    };
    match blacklist.lock() {
        Ok(mut guard) => match guard.iter_mut().find(|existing| {
            existing.provider_id == entry.provider_id
                && existing.logical_model == entry.logical_model
        }) {
            Some(existing) => *existing = entry,
            None => guard.push(entry),
        },
        Err(_) => log::error!("拉黑列表锁中毒,本次写入放弃"),
    }
}

/// 当前拉黑快照。过期由 T4 在 candidates_for 里读的时候判 `until_ms > now`,
/// 这里不做清理(§6.4 第 2 条:绝不加定时器)。
fn blacklist_snapshot(blacklist: &Mutex<Vec<Blacklisted>>) -> Vec<Blacklisted> {
    match blacklist.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            log::error!("拉黑列表锁中毒,按空列表处理");
            Vec::new()
        }
    }
}

/// FailureKind → 库里 failure_kind 的字符串。T5 只定了枚举,字符串取值由 T6
/// 定:与变体名一致的 snake_case。`auth_unavailable`(§4.3)不经过 classify,
/// 在调用处直接写。
fn failure_kind_db_value(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Connect => "connect",
        FailureKind::Timeout => "timeout",
        FailureKind::RateLimited => "rate_limited",
        FailureKind::ServerError => "server_error",
        FailureKind::ModelNotFound => "model_not_found",
        FailureKind::RequestRejected => "request_rejected",
        FailureKind::StreamBroken => "stream_broken",
    }
}

/// 一次尝试的记录参数(单独成结构体,避免 8 个参数触发 clippy)。
struct AttemptRecord<'a> {
    started_at: i64,
    logical_model: &'a str,
    provider_id: &'a str,
    outcome: AttemptOutcome,
    failure_kind: Option<&'a str>,
    http_status: Option<u16>,
    duration_ms: Option<i64>,
}

/// 记一次尝试。写失败只记日志——转发服务不能因为记账失败而拒绝用户请求。
/// input/output token 暂记 None:任务书没要求解析 SSE 里的 usage(那是 17′
/// 的分账语义),留给后续任务。
fn record_attempt(db: &Database, record: AttemptRecord<'_>) {
    let attempt = RouterAttempt {
        started_at: record.started_at,
        logical_model: record.logical_model.to_string(),
        provider_id: record.provider_id.to_string(),
        outcome: record.outcome,
        failure_kind: record.failure_kind.map(str::to_string),
        http_status: record.http_status,
        input_tokens: None,
        output_tokens: None,
        duration_ms: record.duration_ms,
    };
    if let Err(error) = db.record_router_attempt(&attempt) {
        log::error!("记录 router attempt 失败: {error}");
    }
}

/// 一次请求的上下文:所有候选共享,队列里真正发出请求前 body 不会丢。
struct RequestContext {
    state: Arc<RouterState>,
    client_headers: HeaderMap,
    logical_model: String,
    providers: HashMap<String, RouterProvider>,
    candidates: Vec<Candidate>,
    /// 请求体超过 4 MB:发出一次后不可重发,classify 时按「已吐字」压制换家。
    oversize: bool,
    buffered_body: Option<Bytes>,
    oversize_head: Option<Bytes>,
    oversize_tail: Option<BodyDataStream>,
    request_start_ms: i64,
}

/// 单个候选尝试的结果。
enum Step {
    /// 2xx:响应已经在向客户端流式透传,整个请求结束。
    Success(Response),
    /// 失败且不再换家:错误已经转成响应,整个请求结束。
    GiveUp(Response),
    /// 失败但换下一家:留下错误摘要,队列走完时兜底返回。
    TryNext(LastError),
    /// 发送前就被跳过(协议不匹配/凭据取不到),不产生「最后错误」。
    Skipped,
}

/// 队列里最后一个失败候选的摘要(§4 步骤 8 用)。换下一家时错误体的剩余部分
/// 已被排干,这里只留片段。
struct LastError {
    provider_id: String,
    result: AttemptResult,
    status: Option<StatusCode>,
    content_type: Option<HeaderValue>,
    snippet: String,
}

/// POST /v1/responses 的处理器。axum 默认每个请求一个 task(§7 崩溃隔离),
/// 本函数里任何错误都转成带说明的响应,不 panic。
async fn handle_responses(
    State(state): State<Arc<RouterState>>,
    client_headers: HeaderMap,
    body: Body,
) -> Response {
    match handle_responses_inner(state, client_headers, body).await {
        Ok(response) => response,
        // 请求本身有问题(不是合法 JSON、缺 model、超限且扫不到 model)是 400,
        // 其余一律 500。
        Err(AppError::InvalidInput(message)) => {
            log::warn!("router 收到无法处理的请求: {message}");
            plain_response(StatusCode::BAD_REQUEST, message)
        }
        Err(error) => internal_error_response(&error),
    }
}

async fn handle_responses_inner(
    state: Arc<RouterState>,
    client_headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    // 1. 读 body(带缓存上限)
    let inbound = read_request_body(body).await?;
    // 2. 逻辑模型
    let logical_model = extract_logical_model(&inbound)?;
    // 3. 该模型的路由 + provider 元数据(拿 base_url 与 wire_api)
    let routes = state.db.list_model_routes(&logical_model)?;
    let providers: HashMap<String, RouterProvider> = state
        .db
        .list_router_providers()?
        .into_iter()
        .map(|provider| (provider.id.clone(), provider))
        .collect();
    // 4. 拉黑 + 模式
    let now_ms = now_millis();
    let blacklist = blacklist_snapshot(&state.blacklist);
    let mode = read_router_mode(&state.db)?;
    // 5. 候选队列(T4)
    let candidates = candidates_for(&routes, &blacklist, &mode, now_ms);
    log::debug!(
        "router 请求: model={logical_model}, 候选 {} 家",
        candidates.len()
    );
    // 6. 队列为空 → 503 + 记 skipped
    if candidates.is_empty() {
        record_attempt(
            &state.db,
            AttemptRecord {
                started_at: now_ms,
                logical_model: &logical_model,
                provider_id: "",
                outcome: AttemptOutcome::Skipped,
                failure_kind: None,
                http_status: None,
                duration_ms: None,
            },
        );
        return Ok(no_candidates_response(&logical_model));
    }
    // 7-8. 依次转发
    let (oversize, buffered_body, oversize_head, oversize_tail) = match inbound {
        InboundBody::Buffered(bytes) => (false, Some(bytes), None, None),
        InboundBody::Oversize { head, tail } => (true, None, Some(head), Some(tail)),
    };
    let ctx = RequestContext {
        state,
        client_headers,
        logical_model,
        providers,
        candidates,
        oversize,
        buffered_body,
        oversize_head,
        oversize_tail,
        request_start_ms: now_ms,
    };
    forward_through_candidates(ctx).await
}

async fn forward_through_candidates(mut ctx: RequestContext) -> Result<Response, AppError> {
    let mut last_failure: Option<LastError> = None;
    let total = ctx.candidates.len();
    for index in 0..total {
        let candidate = ctx.candidates[index].clone();
        match try_one_candidate(&mut ctx, &candidate, index + 1 == total).await? {
            Step::Success(response) | Step::GiveUp(response) => return Ok(response),
            Step::TryNext(last) => last_failure = Some(last),
            Step::Skipped => {}
        }
    }
    // 队列走完仍未成功:返回最后一次的错误;一次都没发出去(全部被跳过)则明确说明。
    match last_failure {
        Some(last) => forward_last_error(last),
        None => Ok(all_candidates_unavailable_response(&ctx.logical_model)),
    }
}

#[allow(clippy::too_many_lines)]
async fn try_one_candidate(
    ctx: &mut RequestContext,
    candidate: &Candidate,
    is_last: bool,
) -> Result<Step, AppError> {
    let state = &ctx.state;
    // provider 元数据。路由表由 T3 保证 provider 存在且启用,这里防御性兜底。
    let Some(provider) = ctx.providers.get(&candidate.provider_id) else {
        log::warn!(
            "候选 {} 不在 router_providers 里,跳过",
            candidate.provider_id
        );
        record_attempt(
            &state.db,
            AttemptRecord {
                started_at: ctx.request_start_ms,
                logical_model: &ctx.logical_model,
                provider_id: &candidate.provider_id,
                outcome: AttemptOutcome::Skipped,
                failure_kind: None,
                http_status: None,
                duration_ms: None,
            },
        );
        return Ok(Step::Skipped);
    };
    // 协议一致性:v1 不做协议转换(§6),wire_api 不匹配的直接跳过并记 skipped。
    if provider.wire_api != WireApi::Responses {
        log::info!(
            "跳过 wire_api 不匹配的候选 {} ({:?})",
            provider.id,
            provider.wire_api
        );
        record_attempt(
            &state.db,
            AttemptRecord {
                started_at: ctx.request_start_ms,
                logical_model: &ctx.logical_model,
                provider_id: &candidate.provider_id,
                outcome: AttemptOutcome::Skipped,
                failure_kind: None,
                http_status: None,
                duration_ms: None,
            },
        );
        return Ok(Step::Skipped);
    }
    // 认证头:要发的那一刻才问,拿到就用,用完不留(§4.3)。
    let auth_headers = match state.auth.headers_for(&candidate.provider_id).await {
        Ok(headers) => headers,
        Err(error) => {
            log::warn!("取 {} 的上游认证头失败: {error}", candidate.provider_id);
            record_attempt(
                &state.db,
                AttemptRecord {
                    started_at: ctx.request_start_ms,
                    logical_model: &ctx.logical_model,
                    provider_id: &candidate.provider_id,
                    outcome: AttemptOutcome::Failed,
                    failure_kind: Some("auth_unavailable"),
                    http_status: None,
                    duration_ms: None,
                },
            );
            return Ok(Step::Skipped);
        }
    };
    // 改写 model,组装出站体。
    let outbound_body = match (&ctx.buffered_body, &ctx.oversize_head) {
        (Some(bytes), _) => {
            OutboundBody::Buffered(rewrite_buffered_body(bytes, &candidate.upstream_model)?)
        }
        (None, Some(head)) => {
            let head = rewrite_model_in_prefix(head, &candidate.upstream_model)?;
            let tail = match ctx.oversize_tail.take() {
                Some(tail) => tail,
                None => {
                    return Err(AppError::InvalidInput(
                        "超限请求体的剩余流已被消耗".to_string(),
                    ));
                }
            };
            OutboundBody::Stream { head, tail }
        }
        (None, None) => {
            return Err(AppError::InvalidInput("请求体状态不一致".to_string()));
        }
    };
    let url = format!("{}/responses", provider.base_url.trim_end_matches('/'));
    let outbound_headers = build_upstream_headers(&ctx.client_headers, &auth_headers);
    let attempt_start = now_millis();
    let send_result = state
        .sender
        .send(OutboundRequest {
            url,
            headers: outbound_headers,
            body: outbound_body,
        })
        .await;
    let (result, parts) = match send_result {
        Ok(response) if (200..=299).contains(&response.status().as_u16()) => {
            return stream_upstream_success(
                state,
                response,
                candidate,
                &ctx.logical_model,
                attempt_start,
            )
            .map(Step::Success);
        }
        Ok(response) => {
            let status = response.status();
            let parts = read_error_parts(response).await;
            let result = AttemptResult::Http {
                status: status.as_u16(),
                body_snippet: parts.snippet.clone(),
            };
            (result, Some(parts))
        }
        Err(SendFailure::FirstByteTimeout) => (AttemptResult::FirstByteTimeout, None),
        Err(SendFailure::Connect) => (AttemptResult::ConnectFailed, None),
    };
    let Some(verdict) = classify(&result, ctx.oversize) else {
        // 非 2xx 必有判定;走到这里说明分类函数契约被破坏,返回错误而不是 panic。
        log::error!("classify 对失败结果未给出判定: {result:?}");
        return Err(AppError::InvalidInput("失败分类缺失判定".to_string()));
    };
    let duration_ms = now_millis() - attempt_start;
    let http_status = match &result {
        AttemptResult::Http { status, .. } => Some(*status),
        _ => None,
    };
    record_attempt(
        &state.db,
        AttemptRecord {
            started_at: attempt_start,
            logical_model: &ctx.logical_model,
            provider_id: &candidate.provider_id,
            outcome: AttemptOutcome::Failed,
            failure_kind: Some(failure_kind_db_value(verdict.kind)),
            http_status,
            duration_ms: Some(duration_ms),
        },
    );
    if let Some(scope) = verdict.blacklist {
        push_blacklist(
            &state.blacklist,
            &candidate.provider_id,
            scope,
            &ctx.logical_model,
            now_millis(),
            verdict.cooldown_secs,
        );
    }
    if verdict.try_next && !is_last {
        let last = match parts {
            Some(parts) => {
                let status = parts.status;
                let content_type = parts.content_type.clone();
                let snippet = parts.snippet.clone();
                drain_error_tail(parts).await;
                LastError {
                    provider_id: candidate.provider_id.clone(),
                    result,
                    status: Some(status),
                    content_type,
                    snippet,
                }
            }
            None => LastError {
                provider_id: candidate.provider_id.clone(),
                result,
                status: None,
                content_type: None,
                snippet: String::new(),
            },
        };
        return Ok(Step::TryNext(last));
    }
    match parts {
        Some(parts) => forward_upstream_error(parts).map(Step::GiveUp),
        None => Ok(Step::GiveUp(gateway_error_response(
            &result,
            &candidate.provider_id,
        ))),
    }
}

/// 2xx:记 attempt(success) 后把响应流式透传给客户端,绝不整体读进内存(§4.1)。
/// 若流在吐字之后中断,再补记一条 failed/stream_broken(不换家、不拉黑,错误
/// 只能以「流到此结束」的形式透传)。
fn stream_upstream_success(
    state: &RouterState,
    response: reqwest::Response,
    candidate: &Candidate,
    logical_model: &str,
    attempt_start: i64,
) -> Result<Response, AppError> {
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    record_attempt(
        &state.db,
        AttemptRecord {
            started_at: attempt_start,
            logical_model,
            provider_id: &candidate.provider_id,
            outcome: AttemptOutcome::Success,
            failure_kind: None,
            http_status: Some(status.as_u16()),
            duration_ms: Some(now_millis() - attempt_start),
        },
    );
    let db = state.db.clone();
    let provider_id = candidate.provider_id.clone();
    let logical_model = logical_model.to_string();
    let status_code = status.as_u16();
    let stream = async_stream::stream! {
        let mut inner = response.bytes_stream();
        let mut broken = false;
        while let Some(chunk) = inner.next().await {
            match chunk {
                Ok(bytes) => {
                    yield Ok::<Bytes, std::convert::Infallible>(bytes);
                }
                Err(error) => {
                    broken = true;
                    log::warn!("上游 {provider_id} 的响应流中断: {error}");
                    break;
                }
            }
        }
        if broken {
            // 已经吐字之后断流:错误只能透传(流到此结束),不换家、不拉黑(T5 判定表第 4 行)。
            record_attempt(
                &db,
                AttemptRecord {
                    started_at: attempt_start,
                    logical_model: &logical_model,
                    provider_id: &provider_id,
                    outcome: AttemptOutcome::Failed,
                    failure_kind: Some("stream_broken"),
                    http_status: Some(status_code),
                    duration_ms: Some(now_millis() - attempt_start),
                },
            );
        }
    };
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|error| AppError::InvalidInput(format!("构建流式响应失败: {error}")))
}

/// 非 2xx 响应的错误体:取一小段给 classify 做「模型不存在」判断,
/// 其余部分原样留给「转发给客户端」或「换家后排干」。
struct UpstreamErrorParts {
    status: StatusCode,
    content_type: Option<HeaderValue>,
    snippet: String,
    tail: Option<Body>,
}

async fn read_error_parts(response: reqwest::Response) -> UpstreamErrorParts {
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let mut stream = response.bytes_stream();
    let mut snippet: Vec<u8> = Vec::new();
    let mut tail: Option<Body> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                log::warn!("读取上游错误体失败: {error}");
                break;
            }
        };
        let room = ERROR_SNIPPET_MAX.saturating_sub(snippet.len());
        if chunk.len() <= room {
            snippet.extend_from_slice(&chunk);
        } else {
            if room > 0 {
                snippet.extend_from_slice(&chunk[..room]);
            }
            tail = Some(Body::from_stream(
                futures::stream::once(
                    async move { Ok::<Bytes, reqwest::Error>(chunk.slice(room..)) },
                )
                .chain(stream),
            ));
            break;
        }
    }
    UpstreamErrorParts {
        status,
        content_type,
        snippet: String::from_utf8_lossy(&snippet).into_owned(),
        tail,
    }
}

/// 排干一个不会再转发的错误体(换下一家时丢弃)。读一点让连接有机会回池,
/// 超过上限就放弃。
async fn drain_error_tail(parts: UpstreamErrorParts) {
    let Some(tail) = parts.tail else {
        return;
    };
    let mut stream = tail.into_data_stream();
    let mut drained = 0usize;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            break;
        };
        drained += chunk.len();
        if drained > ERROR_DRAIN_MAX {
            break;
        }
    }
}

/// 把上游的错误响应原样透传给客户端(不换家,或已经是最后一家)。
fn forward_upstream_error(parts: UpstreamErrorParts) -> Result<Response, AppError> {
    let mut builder = Response::builder().status(parts.status);
    if let Some(content_type) = parts.content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    let snippet = Bytes::from(parts.snippet.into_bytes());
    let body = match parts.tail {
        Some(tail) => Body::from_stream(
            futures::stream::once(async move { Ok::<Bytes, axum::Error>(snippet) })
                .chain(tail.into_data_stream()),
        ),
        None => Body::from(snippet),
    };
    builder
        .body(body)
        .map_err(|error| AppError::InvalidInput(format!("构建错误转发响应失败: {error}")))
}

/// 队列走完时转发最后一个失败:有 HTTP 响应的透传片段,连不上/首字节超时
/// 合成 502(没有上游响应可透传)。
fn forward_last_error(last: LastError) -> Result<Response, AppError> {
    match (last.status, last.content_type) {
        (Some(status), content_type) => {
            let mut builder = Response::builder().status(status);
            if let Some(content_type) = content_type {
                builder = builder.header(header::CONTENT_TYPE, content_type);
            }
            builder
                .body(Body::from(last.snippet))
                .map_err(|error| AppError::InvalidInput(format!("构建错误转发响应失败: {error}")))
        }
        (None, _) => Ok(gateway_error_response(&last.result, &last.provider_id)),
    }
}

/// 连不上/首字节超时没有上游响应可透传,合成 502(§4 步骤 8)。
fn gateway_error_response(result: &AttemptResult, provider_id: &str) -> Response {
    let message = match result {
        AttemptResult::ConnectFailed => format!("上游 {provider_id} 连接失败"),
        AttemptResult::FirstByteTimeout => format!("上游 {provider_id} 首字节超时"),
        _ => "上游请求失败".to_string(),
    };
    plain_response(StatusCode::BAD_GATEWAY, message)
}

/// 候选队列为空(§4 步骤 6)。
fn no_candidates_response(logical_model: &str) -> Response {
    plain_response(
        StatusCode::SERVICE_UNAVAILABLE,
        format!(
            "模型 {logical_model} 没有可用候选:可能没有配置路由,或所有 provider 都还在冷却期内"
        ),
    )
}

/// 队列里所有候选都被跳过(协议不匹配/凭据不可用),一次都没发出去。
fn all_candidates_unavailable_response(logical_model: &str) -> Response {
    plain_response(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("模型 {logical_model} 的所有候选都被跳过(协议不匹配或凭据不可用)"),
    )
}

fn internal_error_response(error: &AppError) -> Response {
    log::error!("本地 router 内部错误: {error}");
    plain_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("本地路由处理失败: {error}"),
    )
}

fn plain_response(status: StatusCode, message: String) -> Response {
    match Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message))
    {
        Ok(response) => response,
        Err(error) => {
            log::error!("构建纯文本响应失败: {error}");
            Response::new(Body::empty())
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{ModelRoute, RouterAuthKind};
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use std::collections::VecDeque;
    use tower::ServiceExt;

    const MODEL: &str = "gpt-5.6-sol";

    fn memory_db() -> Arc<Database> {
        Arc::new(Database::memory().unwrap())
    }

    fn seed_route(
        db: &Database,
        provider_id: &str,
        wire_api: WireApi,
        priority: i64,
        upstream_model: &str,
    ) {
        db.upsert_router_provider(&RouterProvider {
            id: provider_id.to_string(),
            display_name: provider_id.to_string(),
            base_url: format!("https://{provider_id}.example/v1"),
            wire_api,
            priority,
            enabled: true,
            auth_kind: RouterAuthKind::None,
            credential_key_id: None,
        })
        .unwrap();
        db.upsert_model_route(&ModelRoute {
            provider_id: provider_id.to_string(),
            logical_model: MODEL.to_string(),
            upstream_model: upstream_model.to_string(),
        })
        .unwrap();
    }

    fn ok_auth() -> Arc<dyn UpstreamAuth> {
        Arc::new(FakeAuth {
            headers: vec![("Authorization".to_string(), "Bearer upstream".to_string())],
            fail_for: None,
        })
    }

    fn fake_sender(script: Vec<ScriptedOutcome>) -> Arc<FakeSender> {
        Arc::new(FakeSender {
            script: Arc::new(Mutex::new(script.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn test_state(
        db: Arc<Database>,
        auth: Arc<dyn UpstreamAuth>,
        sender: Arc<FakeSender>,
    ) -> Arc<RouterState> {
        Arc::new(RouterState {
            db,
            auth,
            sender,
            blacklist: Mutex::new(Vec::new()),
        })
    }

    fn test_router(state: Arc<RouterState>) -> Router {
        build_router(state)
    }

    fn json_body(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    /// 以 JSON body 打 POST /v1/responses,返回 (状态, 响应头, 响应体)。
    async fn post_json(
        router: Router,
        body_bytes: Vec<u8>,
        extra_headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Bytes) {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::from(body_bytes)).unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 64 * 1024 * 1024)
            .await
            .unwrap();
        (status, headers, body)
    }

    /// 读回 router_attempts:(provider_id, outcome 原文, failure_kind)。
    fn attempts(db: &Database) -> Vec<(String, String, Option<String>)> {
        let conn = db.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT provider_id, outcome, failure_kind FROM router_attempts ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    struct FakeAuth {
        headers: Vec<(String, String)>,
        fail_for: Option<String>,
    }

    impl UpstreamAuth for FakeAuth {
        fn headers_for<'a>(
            &'a self,
            provider_id: &'a str,
        ) -> BoxFuture<'a, Result<Vec<(String, String)>, AppError>> {
            Box::pin(async move {
                if self.fail_for.as_deref() == Some(provider_id) {
                    return Err(AppError::Message(format!("{provider_id} 没有可用凭据")));
                }
                Ok(self.headers.clone())
            })
        }
    }

    enum ScriptedOutcome {
        Respond {
            status: u16,
            headers: Vec<(String, String)>,
            body: Vec<u8>,
        },
        Fail(SendFailure),
    }

    struct FakeSender {
        script: Arc<Mutex<VecDeque<ScriptedOutcome>>>,
        requests: Arc<Mutex<Vec<OutboundRequest>>>,
    }

    impl UpstreamSender for FakeSender {
        fn send(
            &self,
            request: OutboundRequest,
        ) -> Pin<Box<dyn Future<Output = Result<reqwest::Response, SendFailure>> + Send>> {
            // trait 要求 'static future:内部状态装进 Arc,克隆进 future 而不是借用 self。
            Box::pin(fake_send(
                self.script.clone(),
                self.requests.clone(),
                request,
            ))
        }
    }

    async fn fake_send(
        script: Arc<Mutex<VecDeque<ScriptedOutcome>>>,
        requests: Arc<Mutex<Vec<OutboundRequest>>>,
        request: OutboundRequest,
    ) -> Result<reqwest::Response, SendFailure> {
        let outcome = match script.lock() {
            Ok(mut script) => script.pop_front(),
            Err(_) => None,
        };
        if let Ok(mut requests) = requests.lock() {
            requests.push(request);
        }
        match outcome {
            None => Err(SendFailure::Connect),
            Some(ScriptedOutcome::Fail(failure)) => Err(failure),
            Some(ScriptedOutcome::Respond {
                status,
                headers,
                body,
            }) => {
                let mut builder = axum::http::Response::builder().status(status);
                for (name, value) in &headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                let response = builder.body(reqwest::Body::from(body)).unwrap();
                Ok(reqwest::Response::from(response))
            }
        }
    }

    // —— 任务书 §8 必测项 ——

    /// 1. model 字段改写:其余字段用 serde_json 整棵树比对,必须一字未动。
    #[test]
    fn rewrite_buffered_body_replaces_only_the_model_field() {
        let body = json_body(json!({
            "model": "gpt-5.6-sol",
            "input": [{"role": "user", "content": "hi"}],
            "stream": true,
            "temperature": 1.0,
        }));
        let rewritten = rewrite_buffered_body(&body, "sol").unwrap();

        let before: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mut expected = before.clone();
        expected["model"] = serde_json::Value::String("sol".to_string());
        let after: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(after, expected, "整棵树对比:除 model 外所有字段一致");
        assert_ne!(before["model"], after["model"]);
    }

    /// 2. body 超过 4 MB 走不缓存路径:出站体是流、只发一次,失败也不换家。
    #[tokio::test]
    async fn oversized_body_takes_streaming_path_without_failover() {
        let db = memory_db();
        seed_route(&db, "packyapi", WireApi::Responses, 1, MODEL);
        let sender = fake_sender(vec![
            ScriptedOutcome::Fail(SendFailure::Connect),
            ScriptedOutcome::Respond {
                status: 200,
                headers: vec![],
                body: b"never reached".to_vec(),
            },
        ]);
        let router = test_router(test_state(db, ok_auth(), sender.clone()));

        let big = "a".repeat(MAX_BUFFERED_BODY + 10_000);
        let (status, _, body) = post_json(
            router,
            json_body(json!({"model": MODEL, "input": big})),
            &[],
        )
        .await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(String::from_utf8_lossy(&body).contains("连接失败"));
        let requests = sender.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "超限体不允许故障转移,只发一次");
        let OutboundBody::Stream { head, .. } = &requests[0].body else {
            panic!("超限体必须走流式出站,而不是整体缓存");
        };
        assert!(std::str::from_utf8(head)
            .unwrap()
            .starts_with("{\"model\":\""));
    }

    /// 3. wire_api 不匹配的候选被跳过,并记了 skipped。
    #[tokio::test]
    async fn wire_api_mismatch_candidate_is_skipped_and_recorded() {
        let db = memory_db();
        seed_route(&db, "chat-only", WireApi::ChatCompletions, 1, "chat-model");
        seed_route(&db, "responses-ok", WireApi::Responses, 2, MODEL);
        let sender = fake_sender(vec![ScriptedOutcome::Respond {
            status: 200,
            headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
            body: b"data: ok\n\n".to_vec(),
        }]);
        let router = test_router(test_state(db.clone(), ok_auth(), sender.clone()));

        let (status, _, _) = post_json(router, json_body(json!({"model": MODEL})), &[]).await;

        assert_eq!(status, StatusCode::OK);
        let requests = sender.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "chat_completions 的候选不该发请求");
        assert_eq!(requests[0].url, "https://responses-ok.example/v1/responses");
        assert_eq!(
            attempts(&db),
            vec![
                ("chat-only".to_string(), "skipped".to_string(), None),
                ("responses-ok".to_string(), "success".to_string(), None),
            ],
        );
    }

    /// 4. 队列为空时返回 503,并记一条 skipped。
    #[tokio::test]
    async fn empty_candidate_queue_returns_503_and_records_skipped() {
        let db = memory_db();
        let sender = fake_sender(vec![]);
        let router = test_router(test_state(db.clone(), ok_auth(), sender));

        let (status, _, body) =
            post_json(router, json_body(json!({"model": "unknown-model"})), &[]).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(String::from_utf8_lossy(&body).contains("unknown-model"));
        assert_eq!(
            attempts(&db),
            vec![("".to_string(), "skipped".to_string(), None)],
        );
    }

    /// 5. 拉黑写入后,同一 (provider, 模型) 在冷却期内不再出现在候选里(与 T4 联测)。
    #[test]
    fn blacklist_write_keeps_route_out_of_candidates_during_cooldown() {
        let blacklist = Mutex::new(Vec::new());
        let now = 1_000_000i64;
        push_blacklist(
            &blacklist,
            "official",
            BlacklistScope::ThisRoute,
            MODEL,
            now,
            60,
        );
        let entries = blacklist_snapshot(&blacklist);
        assert_eq!(
            entries,
            vec![Blacklisted {
                provider_id: "official".to_string(),
                logical_model: Some(MODEL.to_string()),
                until_ms: now + 60_000,
            }],
        );

        let routes = vec![
            ModelRoute {
                provider_id: "official".to_string(),
                logical_model: MODEL.to_string(),
                upstream_model: MODEL.to_string(),
            },
            ModelRoute {
                provider_id: "packyapi".to_string(),
                logical_model: MODEL.to_string(),
                upstream_model: "sol".to_string(),
            },
        ];
        // 冷却期内:official 被跳过。
        let candidates = candidates_for(&routes, &entries, &RouteMode::Auto, now + 59_999);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider_id, "packyapi");
        // 到期边界(until_ms == now 视为过期,读时判断,无需清理任务):恢复。
        let candidates = candidates_for(&routes, &entries, &RouteMode::Auto, now + 60_000);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].provider_id, "official");
        // provider 级拉黑:整个 provider 都被跳过。
        // 取 now + 61_000:official 的行级拉黑(60 秒)已过期,packyapi 的
        // provider 级拉黑(600 秒)仍在冷却期内,只剩 official 一家。
        push_blacklist(
            &blacklist,
            "packyapi",
            BlacklistScope::WholeProvider,
            MODEL,
            now,
            600,
        );
        let entries = blacklist_snapshot(&blacklist);
        let candidates = candidates_for(&routes, &entries, &RouteMode::Auto, now + 61_000);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider_id, "official");
        // 同一 (provider, 模型) 重复写入只保留最新一条,列表不无限增长。
        push_blacklist(
            &blacklist,
            "official",
            BlacklistScope::ThisRoute,
            MODEL,
            now + 1_000,
            60,
        );
        assert_eq!(blacklist_snapshot(&blacklist).len(), 2);
    }

    /// 6. 客户端来的 Authorization 不出现在转出去的头里,UpstreamAuth 的头出现。
    #[tokio::test]
    async fn client_credentials_are_stripped_and_upstream_auth_headers_are_injected() {
        let db = memory_db();
        seed_route(&db, "packyapi", WireApi::Responses, 1, MODEL);
        let sender = fake_sender(vec![ScriptedOutcome::Respond {
            status: 200,
            headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
            body: b"data: ok\n\n".to_vec(),
        }]);
        let auth = Arc::new(FakeAuth {
            headers: vec![
                (
                    "Authorization".to_string(),
                    "Bearer upstream-secret".to_string(),
                ),
                ("X-Custom".to_string(), "v".to_string()),
            ],
            fail_for: None,
        });
        let router = test_router(test_state(db, auth, sender.clone()));

        let (status, _, body) = post_json(
            router,
            json_body(json!({"model": MODEL})),
            &[
                ("authorization", "Bearer client-secret"),
                ("cookie", "session=abc"),
                ("x-api-key", "client-key"),
                ("openai-organization", "org-1"),
                ("x-unrelated", "zzz"),
            ],
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"data: ok\n\n");
        let requests = sender.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let forwarded: HashMap<String, String> = requests[0].headers.iter().cloned().collect();
        assert_eq!(
            forwarded.len(),
            4,
            "转出去的头只能是白名单 + 注入头: {forwarded:?}"
        );
        assert_eq!(
            forwarded.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            forwarded.get("accept").map(String::as_str),
            Some("text/event-stream")
        );
        assert_eq!(
            forwarded.get("Authorization").map(String::as_str),
            Some("Bearer upstream-secret")
        );
        assert_eq!(forwarded.get("X-Custom").map(String::as_str), Some("v"));
    }

    /// 7. router.mode = "manual:packyapi" 时只试那一家;垃圾字符串退回 auto。
    #[tokio::test]
    async fn manual_mode_tries_only_selected_provider_and_garbage_falls_back_to_auto() {
        let db = memory_db();
        seed_route(&db, "official", WireApi::Responses, 1, MODEL);
        seed_route(&db, "packyapi", WireApi::Responses, 2, "sol");

        // manual:packyapi → 只发 packyapi 一家。
        db.set_setting("router.mode", "manual:packyapi").unwrap();
        let sender_a = fake_sender(vec![ScriptedOutcome::Respond {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
        }]);
        let router_a = test_router(test_state(db.clone(), ok_auth(), sender_a.clone()));
        let (status, _, _) = post_json(router_a, json_body(json!({"model": MODEL})), &[]).await;
        assert_eq!(status, StatusCode::OK);
        // 单独开一层作用域:守卫必须在下面那个 await 之前释放,
        // 否则 clippy::await_holding_lock 会拦(--all-targets 下)。
        {
            let requests = sender_a.requests.lock().unwrap();
            assert_eq!(requests.len(), 1, "手动模式只试选中的那一家");
            assert!(requests[0].url.contains("packyapi.example"));
        }

        // 垃圾字符串 → 退回 auto:按优先级依次试,第一家失败换第二家。
        db.set_setting("router.mode", "whatever").unwrap();
        let sender_b = fake_sender(vec![
            ScriptedOutcome::Fail(SendFailure::Connect),
            ScriptedOutcome::Respond {
                status: 200,
                headers: vec![],
                body: b"ok".to_vec(),
            },
        ]);
        let router_b = test_router(test_state(db.clone(), ok_auth(), sender_b.clone()));
        let (status, _, _) = post_json(router_b, json_body(json!({"model": MODEL})), &[]).await;
        assert_eq!(status, StatusCode::OK);
        let requests = sender_b.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "auto 模式下失败要换下一家");
        assert!(requests[0].url.contains("official.example"));
        assert!(requests[1].url.contains("packyapi.example"));
    }

    // —— 补充:任务书流程里必须有的行为,顺手盯住 ——

    /// headers_for 返回 Err:记 failed/auth_unavailable,继续下一个候选(§4.3)。
    #[tokio::test]
    async fn auth_failure_records_auth_unavailable_and_tries_next_provider() {
        let db = memory_db();
        seed_route(&db, "official", WireApi::Responses, 1, MODEL);
        seed_route(&db, "packyapi", WireApi::Responses, 2, "sol");
        let sender = fake_sender(vec![ScriptedOutcome::Respond {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
        }]);
        let auth = Arc::new(FakeAuth {
            headers: vec![],
            fail_for: Some("official".to_string()),
        });
        let router = test_router(test_state(db.clone(), auth, sender.clone()));

        let (status, _, _) = post_json(router, json_body(json!({"model": MODEL})), &[]).await;

        assert_eq!(status, StatusCode::OK);
        let requests = sender.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "official 拿不到凭据不该发请求");
        assert!(requests[0].url.contains("packyapi.example"));
        assert_eq!(
            attempts(&db),
            vec![
                (
                    "official".to_string(),
                    "failed".to_string(),
                    Some("auth_unavailable".to_string()),
                ),
                ("packyapi".to_string(), "success".to_string(), None),
            ],
        );
    }

    /// 队列走完仍未成功:返回最后一次的错误(§4 步骤 8)。
    #[tokio::test]
    async fn queue_exhausted_returns_the_last_error_response() {
        let db = memory_db();
        seed_route(&db, "official", WireApi::Responses, 1, MODEL);
        seed_route(&db, "packyapi", WireApi::Responses, 2, "sol");
        let sender = fake_sender(vec![
            ScriptedOutcome::Respond {
                status: 500,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: br#"{"error":"first"}"#.to_vec(),
            },
            ScriptedOutcome::Respond {
                status: 503,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: br#"{"error":"second"}"#.to_vec(),
            },
        ]);
        let router = test_router(test_state(db.clone(), ok_auth(), sender.clone()));

        let (status, headers, body) =
            post_json(router, json_body(json!({"model": MODEL})), &[]).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            headers
                .get("content-type")
                .map(|value| value.to_str().unwrap()),
            Some("application/json")
        );
        assert_eq!(body.as_ref(), br#"{"error":"second"}"#.as_slice());
        let requests = sender.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            attempts(&db),
            vec![
                (
                    "official".to_string(),
                    "failed".to_string(),
                    Some("server_error".to_string()),
                ),
                (
                    "packyapi".to_string(),
                    "failed".to_string(),
                    Some("server_error".to_string()),
                ),
            ],
        );
    }

    /// 只注册 POST /v1/responses:其余路径一律 404,少了 /v1 前缀也不行。
    #[tokio::test]
    async fn unknown_paths_return_404() {
        let db = memory_db();
        let router = test_router(test_state(db, ok_auth(), fake_sender(vec![])));

        for path in ["/responses", "/v1/other", "/"] {
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .body(Body::empty())
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "路径 {path} 必须 404"
            );
        }
    }

    /// router.mode 的解析契约(§4.4):读不到、空串、格式不认识一律 Auto。
    #[test]
    fn parse_router_mode_matches_contract() {
        assert_eq!(parse_router_mode(None), RouteMode::Auto);
        assert_eq!(parse_router_mode(Some("auto")), RouteMode::Auto);
        assert_eq!(parse_router_mode(Some("")), RouteMode::Auto);
        assert_eq!(parse_router_mode(Some("whatever")), RouteMode::Auto);
        assert_eq!(parse_router_mode(Some("manual:")), RouteMode::Auto);
        assert_eq!(
            parse_router_mode(Some("manual:packyapi")),
            RouteMode::Manual {
                provider_id: "packyapi".to_string()
            }
        );
    }

    /// 前缀扫描:正常命中、转义、值截断、字符串内容里的 "model" 不误命中。
    #[test]
    fn locate_model_string_in_prefix_handles_escapes_and_boundaries() {
        let head = br#"{"model":"gpt-5.6-sol","input":"x"#;
        let (value, start, end) = locate_model_string_in_prefix(head).unwrap();
        assert_eq!(value, "gpt-5.6-sol");
        assert_eq!(&head[..start], br#"{"model":""#.as_slice());
        assert_eq!(head[end], b'"');

        // 值里带转义引号:取第一个未转义引号结束。
        let (value, _, _) = locate_model_string_in_prefix(br#"{"model":"a\"b"}"#).unwrap();
        assert_eq!(value, "a\"b");

        // model 不是第一个键也能找到。
        let (value, _, _) =
            locate_model_string_in_prefix(br#"{"stream":true,"model":"sol"}"#).unwrap();
        assert_eq!(value, "sol");

        // 值在边界处被截断 → None。
        assert_eq!(locate_model_string_in_prefix(br#"{"model":"abc"#), None);

        // 字符串内容里的 "model" 因为引号被转义而不会误命中。
        assert_eq!(
            locate_model_string_in_prefix(br#"{"input":"the \"model\" word"}"#),
            None
        );
    }

    /// 超限体改 model:字节级拼接,其余部分逐字节不动。
    #[test]
    fn rewrite_model_in_prefix_splices_new_model_keeping_rest_byte_identical() {
        let head = br#"{"model":"gpt-5.6-sol","input":"hello","stream":true"#;
        let rewritten = rewrite_model_in_prefix(head, "sol").unwrap();
        assert_eq!(
            rewritten.as_ref(),
            br#"{"model":"sol","input":"hello","stream":true"#.as_slice()
        );
        assert!(rewrite_model_in_prefix(br#"{"stream":true}"#, "sol").is_err());
    }

    /// 绑定失败要返回 Err(交给调用方提示),且固定 127.0.0.1。
    #[tokio::test]
    async fn bind_loopback_fails_when_port_already_bound() {
        let first = bind_loopback(0).await.unwrap();
        let address = first.local_addr().unwrap();
        assert_eq!(address.ip(), std::net::IpAddr::from([127, 0, 0, 1]));
        assert!(bind_loopback(address.port()).await.is_err());
    }
}
