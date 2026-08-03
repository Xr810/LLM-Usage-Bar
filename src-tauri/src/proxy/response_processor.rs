//! 响应处理器模块
//!
//! 统一处理流式和非流式 API 响应

use super::{
    content_encoding::{decompress_body, get_content_encoding},
    forwarder::ActiveConnectionGuard,
    handler_config::{StreamUsageEventFilter, UsageParserConfig},
    handler_context::{RequestContext, StreamingTimeoutConfig},
    hyper_client::ProxyResponse,
    provider_router::BindingPricingOverride,
    server::ProxyState,
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    usage::logger::UsageLogger,
    ProxyError,
};
use crate::credentials::CredentialExposureGuardSet as CredentialExposureGuard;
use crate::database::PRICING_SOURCE_REQUEST;
use crate::usage::domain::TokenSource;
use crate::usage::ingestion::{FrozenUsageProviderContext, LegacyLogInput, UsageIngestionInput};
use crate::usage::metering::cost_parser::{
    extract_upstream_cost, extract_upstream_cost_from_events, UpstreamCost,
};
use crate::usage::metering::parser::TokenUsage;
use axum::http::{header::HeaderMap, HeaderName};
use axum::response::{IntoResponse, Response};
use bytes::{Bytes, BytesMut};
use futures::stream::{Stream, StreamExt};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::sync::Mutex;

// ============================================================================
// 响应头处理
// ============================================================================

/// RFC 2616 / RFC 7230 中定义的不应被代理继续转发的响应头。
const HOP_BY_HOP_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// 移除响应侧 hop-by-hop 头，以及 `Connection` 中点名的扩展头。
pub(crate) fn strip_hop_by_hop_response_headers(headers: &mut HeaderMap) {
    let connection_listed_headers: Vec<HeaderName> = headers
        .get_all(axum::http::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter_map(|name| HeaderName::from_bytes(name.as_bytes()).ok())
        .collect();

    for name in HOP_BY_HOP_RESPONSE_HEADERS {
        headers.remove(*name);
    }

    for name in connection_listed_headers {
        headers.remove(name);
    }
}

/// Remove upstream-reflected copies of the protected binding key before any
/// response header reaches the local client. Header names are checked too;
/// clients and upstreams may use arbitrary extension fields.
pub(crate) fn strip_credential_bearing_response_headers(
    headers: &mut HeaderMap,
    credential_guard: &CredentialExposureGuard,
) {
    let names = headers
        .iter()
        .filter(|(name, value)| {
            credential_guard.contains(name.as_str())
                || value
                    .to_str()
                    .map(|value| credential_guard.contains(value))
                    .unwrap_or_else(|_| credential_guard.contains_bytes(value.as_bytes()))
        })
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for name in names {
        headers.remove(name);
    }
}

/// 移除在重建响应体后会失真的实体头。
pub(crate) fn strip_entity_headers_for_rebuilt_body(headers: &mut HeaderMap) {
    headers.remove(axum::http::header::CONTENT_ENCODING);
    headers.remove(axum::http::header::CONTENT_LENGTH);
    headers.remove(axum::http::header::TRANSFER_ENCODING);
}

/// 读取响应体并在需要时解压，确保 headers 与返回 body 一致。
///
/// `body_timeout`: 整包超时。当非零时用 `tokio::time::timeout` 包住 `.bytes()` 调用，
/// 防止上游发完响应头后卡住 body 导致请求永远挂住。
/// 传入 `Duration::ZERO` 表示不启用超时（故障转移关闭时）。
pub(crate) async fn read_decoded_body(
    response: ProxyResponse,
    tag: &str,
    body_timeout: Duration,
) -> Result<(HeaderMap, http::StatusCode, Bytes), ProxyError> {
    let mut headers = response.headers().clone();
    let status = response.status();
    let raw_bytes = if body_timeout.is_zero() {
        response.bytes().await?
    } else {
        tokio::time::timeout(body_timeout, response.bytes())
            .await
            .map_err(|_| {
                ProxyError::Timeout(format!(
                    "响应体读取超时: {}s（上游发完响应头后 body 未到达）",
                    body_timeout.as_secs()
                ))
            })??
    };

    log::debug!(
        "[{tag}] 已接收上游响应体: status={}, bytes={}, headers={}",
        status.as_u16(),
        raw_bytes.len(),
        format_headers(&headers)
    );

    let mut body_bytes = raw_bytes.clone();
    let mut decoded = false;

    if let Some(encoding) = get_content_encoding(&headers) {
        log::debug!("[{tag}] 尝试解压非流式响应；编码值已省略");
        match decompress_body(&encoding, &raw_bytes) {
            Ok(Some(decompressed)) => {
                body_bytes = Bytes::from(decompressed);
                decoded = true;
            }
            Ok(None) => {
                log::warn!("[{tag}] opaque upstream response encoding rejected");
                return Err(ProxyError::UpstreamResponseRejected);
            }
            Err(_) => {
                log::warn!("[{tag}] upstream response decompression failed; details omitted");
                return Err(ProxyError::UpstreamResponseRejected);
            }
        }
    }

    if decoded {
        strip_entity_headers_for_rebuilt_body(&mut headers);
    }

    Ok((headers, status, body_bytes))
}

// ============================================================================
// 公共接口
// ============================================================================

/// 检测响应是否为 SSE 流式响应
#[inline]
pub fn is_sse_response(response: &ProxyResponse) -> bool {
    response.is_sse()
}

pub(crate) fn reject_credential_bearing_response_body(
    body: &[u8],
    guard: &CredentialExposureGuard,
) -> Result<(), ProxyError> {
    let semantic_match = serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|value| guard.contains_json_value(&value));
    if guard.contains_bytes(body) || semantic_match {
        log::warn!("Upstream response omitted because it repeated protected credential material");
        Err(ProxyError::UpstreamResponseRejected)
    } else {
        Ok(())
    }
}

const MAX_CREDENTIAL_GUARD_PENDING_BYTES: usize = 8 * 1024 * 1024;

fn credential_in_semantic_sse_block(
    block: &str,
    is_first_block: bool,
    guard: &CredentialExposureGuard,
    semantic_scanner: &mut crate::credentials::CredentialSemanticStreamScannerSet,
) -> bool {
    // The SSE stream grammar permits one UTF-8 BOM at the beginning of the
    // stream. Keep it in the quarantined raw bytes, but ignore it for parsing
    // the first field so a protected prefix cannot hide in that first event.
    let block = if is_first_block {
        block.strip_prefix('\u{feff}').unwrap_or(block)
    } else {
        block
    };
    if guard.contains_bytes(block.as_bytes()) {
        return true;
    }
    let data = block
        .split(['\r', '\n'])
        .filter_map(|line| strip_sse_field(line, "data"))
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return false;
    }
    serde_json::from_str::<Value>(&data)
        .ok()
        .is_some_and(|value| semantic_scanner.push_json_value(&value))
}

/// Inspect an upstream byte stream before transformers, caches, usage parsers,
/// or the local client can observe each chunk. The stateful scanner retains
/// chunk-boundary context for raw and nested URL/form-encoded credentials.
pub(crate) fn guard_credential_response_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    guard: CredentialExposureGuard,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut scanner = guard.stream_scanner();
        let mut semantic_scanner = guard.semantic_stream_scanner();
        let mut semantic_buffer = String::new();
        let mut semantic_utf8_remainder = Vec::new();
        let mut is_first_semantic_block = true;
        let mut pending = BytesMut::new();
        tokio::pin!(stream);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if pending.len().saturating_add(bytes.len())
                        > MAX_CREDENTIAL_GUARD_PENDING_BYTES
                    {
                        pending.clear();
                        log::warn!("Upstream stream stopped because its credential-inspection window exceeded the limit");
                        yield Err(std::io::Error::other("upstream response rejected"));
                        return;
                    }
                    pending.extend_from_slice(&bytes);
                    append_utf8_safe(
                        &mut semantic_buffer,
                        &mut semantic_utf8_remainder,
                        &bytes,
                    );
                    let mut semantic_match = false;
                    while let Some(block) = take_sse_block(&mut semantic_buffer) {
                        let is_first_block = std::mem::replace(
                            &mut is_first_semantic_block,
                            false,
                        );
                        if credential_in_semantic_sse_block(
                            &block,
                            is_first_block,
                            &guard,
                            &mut semantic_scanner,
                        ) {
                            semantic_match = true;
                            break;
                        }
                    }
                    if scanner.push(&bytes) || semantic_match {
                        pending.clear();
                        log::warn!("Upstream stream stopped because it repeated protected credential material");
                        yield Err(std::io::Error::other("upstream response rejected"));
                        return;
                    }
                    // Release only at a complete SSE event boundary where no
                    // normalized semantic channel ends in a protected-key
                    // prefix. This preserves streaming while quarantining the
                    // exact fragments that could combine with a future event.
                    if semantic_buffer.is_empty()
                        && semantic_utf8_remainder.is_empty()
                        && !semantic_scanner.has_partial_match()
                        && !pending.is_empty()
                    {
                        yield Ok(pending.split().freeze());
                    }
                }
                Err(_) => {
                    pending.clear();
                    yield Err(std::io::Error::other("upstream stream failed"));
                    return;
                }
            }
        }
        if !semantic_utf8_remainder.is_empty() {
            pending.clear();
            yield Err(std::io::Error::other("upstream response rejected"));
            return;
        }
        if !semantic_buffer.is_empty()
            && credential_in_semantic_sse_block(
                &semantic_buffer,
                is_first_semantic_block,
                &guard,
                &mut semantic_scanner,
            )
        {
            pending.clear();
            yield Err(std::io::Error::other("upstream response rejected"));
            return;
        }
        // A suffix that is only a key prefix or incomplete escape is safe at
        // EOF: no future event can complete it. Release the quarantined bytes
        // after the final full-match checks above.
        if !pending.is_empty() {
            yield Ok(pending.freeze());
        }
    }
}

/// 处理流式响应
pub async fn handle_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    connection_guard: Option<ActiveConnectionGuard>,
) -> Response {
    let status = response.status();
    log::debug!(
        "[{}] 已接收上游流式响应: status={}, headers={}",
        ctx.tag,
        status.as_u16(),
        format_headers(response.headers())
    );
    // A compressed stream cannot be inspected before egress. Reject it rather
    // than letting a protected credential bypass the stateful scanner.
    if get_content_encoding(response.headers()).is_some() {
        log::warn!("[{}] compressed upstream stream rejected", ctx.tag);
        return ProxyError::UpstreamResponseRejected.into_response();
    }

    let mut response_headers = response.headers().clone();
    strip_hop_by_hop_response_headers(&mut response_headers);
    strip_credential_bearing_response_headers(
        &mut response_headers,
        ctx.credential_exposure_guard(),
    );

    let mut builder = axum::response::Response::builder().status(status);

    // 复制响应头
    for (key, value) in &response_headers {
        builder = builder.header(key, value);
    }

    // 创建字节流
    let stream = guard_credential_response_stream(
        response.bytes_stream(),
        ctx.credential_exposure_guard().clone(),
    );

    // 创建使用量收集器；关闭 usage logging 时不要在流式热路径上解析每个 SSE event。
    let usage_collector = create_usage_collector(ctx, state, status.as_u16(), parser_config);

    // 获取流式超时配置
    let timeout_config = ctx.streaming_timeout_config();

    // 创建带日志和超时的透传流
    let logged_stream = create_logged_passthrough_stream(
        stream,
        ctx.tag,
        usage_collector,
        timeout_config,
        connection_guard,
    );

    let body = axum::body::Body::from_stream(logged_stream);
    match builder.body(body) {
        Ok(resp) => resp,
        Err(e) => {
            log::error!("[{}] 构建流式响应失败: {e}", ctx.tag);
            ProxyError::Internal(format!("Failed to build streaming response: {e}")).into_response()
        }
    }
}

/// 处理非流式响应
pub async fn handle_non_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    // guard 在函数 scope 内持有，整包响应读取完成后随函数返回一并 drop
    _connection_guard: Option<ActiveConnectionGuard>,
) -> Result<Response, ProxyError> {
    // 整包超时：仅在故障转移开启且配置值非零时生效
    let body_timeout =
        if ctx.app_config.auto_failover_enabled && ctx.app_config.non_streaming_timeout > 0 {
            Duration::from_secs(ctx.app_config.non_streaming_timeout as u64)
        } else {
            Duration::ZERO
        };
    let (mut response_headers, status, body_bytes) =
        read_decoded_body(response, ctx.tag, body_timeout).await?;
    reject_credential_bearing_response_body(&body_bytes, ctx.credential_exposure_guard())?;
    strip_hop_by_hop_response_headers(&mut response_headers);
    strip_credential_bearing_response_headers(
        &mut response_headers,
        ctx.credential_exposure_guard(),
    );

    // 解析并记录使用量。关闭 usage logging 时直接跳过，避免非流式响应整包 JSON parse。
    if usage_logging_enabled(state) {
        if let Ok(json_value) = serde_json::from_slice::<Value>(&body_bytes) {
            let upstream_correlation_id = ctx
                .credential_exposure_guard()
                .redact_option(upstream_correlation_id_from_body(&json_value));
            let upstream_cost = validated_upstream_cost(
                extract_upstream_cost(&json_value),
                &ctx.usage_provider_id,
                upstream_correlation_id.as_deref().unwrap_or("unknown"),
            );
            // Invalid explicit cost is a diagnostic ingestion failure. Keep the
            // upstream response intact but do not downgrade the event to estimated.
            if let (Some(upstream_cost), Some(usage)) = (
                upstream_cost.clone(),
                (parser_config.response_parser)(&json_value),
            ) {
                // 归因优先级：usage 解析出的模型 → 响应 model 字段 → 映射后的出站
                // 模型（路由接管真值）→ 客户端请求模型。空字符串视为缺失。
                let model = usage
                    .model
                    .clone()
                    .filter(|m| !m.is_empty())
                    .or_else(|| {
                        json_value
                            .get("model")
                            .and_then(|m| m.as_str())
                            .filter(|m| !m.is_empty())
                            .map(str::to_string)
                    })
                    .or_else(|| ctx.outbound_model.clone())
                    .unwrap_or_else(|| ctx.request_model.clone());

                spawn_log_usage(
                    state,
                    ctx,
                    UsageLogParams {
                        usage,
                        model,
                        status_code: status.as_u16(),
                        is_streaming: false,
                        upstream_cost,
                        upstream_correlation_id,
                    },
                );
            } else if let Some(upstream_cost) = upstream_cost {
                let model = json_value
                    .get("model")
                    .and_then(|m| m.as_str())
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .or_else(|| ctx.outbound_model.clone())
                    .unwrap_or_else(|| ctx.request_model.clone());
                spawn_log_usage(
                    state,
                    ctx,
                    UsageLogParams {
                        usage: TokenUsage::default(),
                        model,
                        status_code: status.as_u16(),
                        is_streaming: false,
                        upstream_cost,
                        upstream_correlation_id,
                    },
                );
                log::debug!(
                    "[{}] 未能解析 usage 信息，跳过记录",
                    parser_config.app_type_str
                );
            }
        } else {
            log::debug!(
                "[{}] <<< 响应 (非 JSON): {} bytes",
                ctx.tag,
                body_bytes.len()
            );
            spawn_log_usage(
                state,
                ctx,
                UsageLogParams {
                    usage: TokenUsage::default(),
                    model: ctx
                        .outbound_model
                        .clone()
                        .unwrap_or_else(|| ctx.request_model.clone()),
                    status_code: status.as_u16(),
                    is_streaming: false,
                    upstream_cost: None,
                    upstream_correlation_id: None,
                },
            );
        }
    } else {
        log::debug!("[{}] usage logging 已关闭，跳过非流式 usage 解析", ctx.tag);
    }

    // 构建响应
    let mut builder = axum::response::Response::builder().status(status);
    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }

    let body = axum::body::Body::from(body_bytes);
    builder.body(body).map_err(|e| {
        log::error!("[{}] 构建响应失败: {e}", ctx.tag);
        ProxyError::Internal(format!("Failed to build response: {e}"))
    })
}

/// 通用响应处理入口
///
/// 根据响应类型自动选择流式或非流式处理
pub async fn process_response(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    connection_guard: Option<ActiveConnectionGuard>,
) -> Result<Response, ProxyError> {
    if is_sse_response(&response) {
        Ok(handle_streaming(response, ctx, state, parser_config, connection_guard).await)
    } else {
        handle_non_streaming(response, ctx, state, parser_config, connection_guard)
            .await
            .map_err(|error| error.redact_credential(ctx.credential_exposure_guard()))
    }
}

// ============================================================================
// SSE 使用量收集器
// ============================================================================

type UsageCallbackWithTiming = Arc<dyn Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static>;

/// SSE 使用量收集器
#[derive(Clone)]
pub struct SseUsageCollector {
    inner: Arc<SseUsageCollectorInner>,
}

struct SseUsageCollectorInner {
    events: Mutex<Vec<Value>>,
    first_event_time: Mutex<Option<std::time::Instant>>,
    first_event_set: AtomicBool,
    start_time: std::time::Instant,
    on_complete: UsageCallbackWithTiming,
    should_collect: Option<StreamUsageEventFilter>,
    finished: AtomicBool,
}

impl SseUsageCollector {
    /// 创建使用量收集器；`should_collect` 用来在 hot path 跳过与 usage 无关的事件。
    pub fn new(
        start_time: std::time::Instant,
        should_collect: Option<StreamUsageEventFilter>,
        callback: impl Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static,
    ) -> Self {
        let on_complete: UsageCallbackWithTiming = Arc::new(callback);
        Self {
            inner: Arc::new(SseUsageCollectorInner {
                events: Mutex::new(Vec::new()),
                first_event_time: Mutex::new(None),
                first_event_set: AtomicBool::new(false),
                start_time,
                on_complete,
                should_collect,
                finished: AtomicBool::new(false),
            }),
        }
    }

    pub fn should_collect(&self, data: &str) -> bool {
        self.inner
            .should_collect
            .map(|filter| filter(data))
            .unwrap_or(true)
    }

    /// 标记首个被收集的 SSE 事件时间，沿用 `first_token_ms` 的既有近似语义。
    async fn mark_first_collected_event_time(&self) {
        if self.inner.first_event_set.load(Ordering::Acquire) {
            return;
        }
        let mut first_time = self.inner.first_event_time.lock().await;
        if first_time.is_none() {
            *first_time = Some(std::time::Instant::now());
            self.inner.first_event_set.store(true, Ordering::Release);
        }
    }

    /// 推送 SSE 事件
    pub async fn push(&self, event: Value) {
        self.mark_first_collected_event_time().await;
        let mut events = self.inner.events.lock().await;
        events.push(event);
    }

    /// 完成收集并触发回调
    pub async fn finish(&self) {
        if self.inner.finished.swap(true, Ordering::SeqCst) {
            return;
        }

        let events = {
            let mut guard = self.inner.events.lock().await;
            std::mem::take(&mut *guard)
        };

        let first_token_ms = {
            let first_time = self.inner.first_event_time.lock().await;
            first_time.map(|t| (t - self.inner.start_time).as_millis() as u64)
        };

        (self.inner.on_complete)(events, first_token_ms);
    }
}

struct SseUsageFinishGuard {
    collector: Option<SseUsageCollector>,
}

impl SseUsageFinishGuard {
    fn new(collector: SseUsageCollector) -> Self {
        Self {
            collector: Some(collector),
        }
    }

    fn disarm(&mut self) {
        self.collector = None;
    }
}

impl Drop for SseUsageFinishGuard {
    fn drop(&mut self) {
        if let Some(collector) = self.collector.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    collector.finish().await;
                });
            } else {
                log::warn!("SSE 用量收尾保护触发时 Tokio runtime 不可用，跳过异步 finish");
            }
        }
    }
}

// ============================================================================
// 内部辅助函数
// ============================================================================

pub(crate) fn upstream_correlation_id_from_body(body: &Value) -> Option<String> {
    body.get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn upstream_correlation_id_from_events(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        event
            .get("response")
            .and_then(|response| response.get("id"))
            .or_else(|| event.get("message").and_then(|message| message.get("id")))
            .or_else(|| event.get("id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

/// Preserve the distinction between a response with no explicit upstream cost
/// and a response whose explicit cost is invalid. The former may be estimated;
/// the latter is a diagnostic ingestion failure and must not silently become an
/// estimated event.
pub(crate) fn validated_upstream_cost(
    result: Result<Option<UpstreamCost>, crate::error::AppError>,
    provider_id: &str,
    request_id: &str,
) -> Option<Option<UpstreamCost>> {
    match result {
        Ok(cost) => Some(cost),
        Err(error) => {
            report_ingestion_failure(provider_id, request_id, &error);
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawSseUsageMetadata {
    pub upstream_cost: Option<UpstreamCost>,
    pub upstream_correlation_id: Option<String>,
    pub invalid_explicit_cost: bool,
}

pub(crate) type SharedRawSseUsageMetadata = Arc<StdMutex<RawSseUsageMetadata>>;

/// Tee an upstream SSE byte stream without changing it while capturing the
/// original cost fields and response ID before any protocol transformer can
/// drop or rewrite them.
pub(crate) fn capture_raw_sse_usage_metadata<E>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
    metadata: SharedRawSseUsageMetadata,
    usage_provider_id: String,
) -> impl Stream<Item = Result<Bytes, E>> + Send
where
    E: std::error::Error + Send + 'static,
{
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            if let Ok(bytes) = &chunk {
                crate::proxy::sse::append_utf8_safe(
                    &mut buffer,
                    &mut utf8_remainder,
                    bytes,
                );
                while let Some(block) = take_sse_block(&mut buffer) {
                    for line in block.split(['\r', '\n']) {
                        let Some(data) = strip_sse_field(line, "data") else {
                            continue;
                        };
                        if data.trim() == "[DONE]" {
                            continue;
                        }
                        let Ok(event) = serde_json::from_str::<Value>(data) else {
                            continue;
                        };
                        let correlation_id =
                            upstream_correlation_id_from_events(std::slice::from_ref(&event));
                        let cost = extract_upstream_cost_from_events(std::slice::from_ref(&event));
                        let mut guard = metadata.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(correlation_id) = correlation_id {
                            guard.upstream_correlation_id = Some(correlation_id);
                        }
                        match cost {
                            Ok(Some(cost)) => guard.upstream_cost = Some(cost),
                            Ok(None) => {}
                            Err(error) => {
                                if !guard.invalid_explicit_cost {
                                    report_ingestion_failure(&usage_provider_id, "upstream", &error);
                                }
                                guard.invalid_explicit_cost = true;
                            }
                        }
                    }
                }
            }
            yield chunk;
        }
    }
}

pub(crate) fn validated_raw_sse_usage_metadata(
    metadata: &SharedRawSseUsageMetadata,
) -> Option<(Option<UpstreamCost>, Option<String>)> {
    let guard = metadata
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (!guard.invalid_explicit_cost).then(|| {
        (
            guard.upstream_cost.clone(),
            guard.upstream_correlation_id.clone(),
        )
    })
}

pub(crate) fn stable_session_id(session_id: &str, client_provided: bool) -> Option<String> {
    client_provided.then(|| session_id.to_string())
}

/// 创建使用量收集器
fn create_usage_collector(
    ctx: &RequestContext,
    state: &ProxyState,
    status_code: u16,
    parser_config: &UsageParserConfig,
) -> Option<SseUsageCollector> {
    let logging_enabled = state
        .config
        .try_read()
        .map(|c| c.enable_logging)
        .unwrap_or(true);
    if !logging_enabled {
        return None;
    }

    let state = state.clone();
    let agent_module_id = ctx.agent_module_id.clone();
    let legacy_provider_id = ctx.legacy_log_provider_id.clone();
    let pricing_override = ctx.pricing_override.clone();
    let usage_provider_id = ctx.usage_provider_id.clone();
    let frozen_provider_context = ctx.frozen_usage_provider_context.clone();
    let request_model = ctx.request_model.clone();
    // 流式事件缺失模型名时的归因兜底：映射后的出站模型（路由接管真值）优先，
    // 其次才是客户端请求别名
    let fallback_model = ctx
        .outbound_model
        .clone()
        .unwrap_or_else(|| ctx.request_model.clone());
    // 用 ctx 的 app_type 而不是 parser_config 的：Claude Desktop 流式透传复用
    // CLAUDE_PARSER_CONFIG（app_type_str="claude"），按 parser_config 记账会把
    // claude-desktop 的行错记到 claude 名下，导致供应商计价覆盖解析不到。
    let app_type_str = ctx.app_type_str;
    let tag = ctx.tag;
    let start_time = ctx.start_time;
    let stream_parser = parser_config.stream_parser;
    let model_extractor = parser_config.model_extractor;
    let session_id = ctx.session_id.clone();
    let session_client_provided = ctx.session_client_provided;
    let credential_guard = ctx.credential_exposure_guard().clone();

    Some(SseUsageCollector::new(
        start_time,
        parser_config.stream_event_filter,
        move |events, first_token_ms| {
            let upstream_correlation_id =
                credential_guard.redact_option(upstream_correlation_id_from_events(&events));
            let Some(upstream_cost) = validated_upstream_cost(
                extract_upstream_cost_from_events(&events),
                &usage_provider_id,
                upstream_correlation_id.as_deref().unwrap_or("unknown"),
            ) else {
                return;
            };
            if let Some(usage) = stream_parser(&events) {
                let model = model_extractor(&events, &fallback_model);
                let latency_ms = start_time.elapsed().as_millis() as u64;

                let state = state.clone();
                let agent_module_id = agent_module_id.clone();
                let legacy_provider_id = legacy_provider_id.clone();
                let pricing_override = pricing_override.clone();
                let usage_provider_id = usage_provider_id.clone();
                let frozen_provider_context = frozen_provider_context.clone();
                let session_id = session_id.clone();
                let request_model = request_model.clone();
                let outbound_model = fallback_model.clone();
                let credential_guard = credential_guard.clone();

                tokio::spawn(async move {
                    ingest_usage_internal(
                        &state,
                        &agent_module_id,
                        &usage_provider_id,
                        frozen_provider_context,
                        &legacy_provider_id,
                        pricing_override,
                        app_type_str,
                        &model,
                        &request_model,
                        &outbound_model,
                        usage,
                        latency_ms,
                        first_token_ms,
                        true, // is_streaming
                        status_code,
                        stable_session_id(&session_id, session_client_provided),
                        Some(session_id),
                        upstream_cost,
                        upstream_correlation_id,
                        credential_guard,
                    )
                    .await;
                });
            } else {
                let model = model_extractor(&events, &fallback_model);
                let latency_ms = start_time.elapsed().as_millis() as u64;
                let state = state.clone();
                let agent_module_id = agent_module_id.clone();
                let legacy_provider_id = legacy_provider_id.clone();
                let pricing_override = pricing_override.clone();
                let usage_provider_id = usage_provider_id.clone();
                let frozen_provider_context = frozen_provider_context.clone();
                let session_id = session_id.clone();
                let request_model = request_model.clone();
                let outbound_model = fallback_model.clone();
                let credential_guard = credential_guard.clone();

                tokio::spawn(async move {
                    ingest_usage_internal(
                        &state,
                        &agent_module_id,
                        &usage_provider_id,
                        frozen_provider_context,
                        &legacy_provider_id,
                        pricing_override,
                        app_type_str,
                        &model,
                        &request_model,
                        &outbound_model,
                        TokenUsage::default(),
                        latency_ms,
                        first_token_ms,
                        true, // is_streaming
                        status_code,
                        stable_session_id(&session_id, session_client_provided),
                        Some(session_id),
                        upstream_cost,
                        upstream_correlation_id,
                        credential_guard,
                    )
                    .await;
                });
                log::debug!("[{tag}] 流式响应缺少 usage 统计，跳过消费记录");
            }
        },
    ))
}

/// 异步记录使用量
struct UsageLogParams {
    usage: TokenUsage,
    model: String,
    status_code: u16,
    is_streaming: bool,
    upstream_cost: Option<UpstreamCost>,
    upstream_correlation_id: Option<String>,
}

fn spawn_log_usage(state: &ProxyState, ctx: &RequestContext, params: UsageLogParams) {
    let UsageLogParams {
        usage,
        model,
        status_code,
        is_streaming,
        upstream_cost,
        upstream_correlation_id,
    } = params;

    // Check enable_logging before spawning the log task
    if let Ok(config) = state.config.try_read() {
        if !config.enable_logging {
            return;
        }
    }

    let state = state.clone();
    let agent_module_id = ctx.agent_module_id.clone();
    let legacy_provider_id = ctx.legacy_log_provider_id.clone();
    let pricing_override = ctx.pricing_override.clone();
    let usage_provider_id = ctx.usage_provider_id.clone();
    let frozen_provider_context = ctx.frozen_usage_provider_context.clone();
    let app_type_str = ctx.app_type_str.to_string();
    let request_model = ctx.request_model.clone();
    // 「按请求计价」模式的锚点：映射后的出站模型，无映射时等于 request_model
    let outbound_model = ctx
        .outbound_model
        .clone()
        .unwrap_or_else(|| ctx.request_model.clone());
    let latency_ms = ctx.latency_ms();
    let session_id = ctx.session_id.clone();
    let stable_session_id = stable_session_id(&session_id, ctx.session_client_provided);
    let credential_guard = ctx.credential_exposure_guard().clone();

    tokio::spawn(async move {
        ingest_usage_internal(
            &state,
            &agent_module_id,
            &usage_provider_id,
            frozen_provider_context,
            &legacy_provider_id,
            pricing_override,
            &app_type_str,
            &model,
            &request_model,
            &outbound_model,
            usage,
            latency_ms,
            None,
            is_streaming,
            status_code,
            stable_session_id,
            Some(session_id),
            upstream_cost,
            upstream_correlation_id,
            credential_guard,
        )
        .await;
    });
}

pub(crate) fn usage_logging_enabled(state: &ProxyState) -> bool {
    state
        .config
        .try_read()
        .map(|config| config.enable_logging)
        .unwrap_or(true)
}

/// 内部使用量记录函数
///
/// `outbound_model` 是「按请求计价」模式的锚点：实际发往上游的模型
/// （路由接管映射后的真值，无映射时等于 request_model）。该模式的语义是
/// 「按代理发出的请求计价、不信任上游回显」，接管场景下发出的请求模型是
/// 映射后的 Y 而非客户端别名 X，按 X 计价会用错定价表行。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn ingest_usage_internal(
    state: &ProxyState,
    agent_module_id: &str,
    usage_provider_id: &str,
    frozen_provider_context: FrozenUsageProviderContext,
    legacy_provider_id: &str,
    pricing_override: BindingPricingOverride,
    app_type: &str,
    model: &str,
    request_model: &str,
    outbound_model: &str,
    usage: TokenUsage,
    latency_ms: u64,
    first_token_ms: Option<u64>,
    is_streaming: bool,
    status_code: u16,
    stable_session_id: Option<String>,
    legacy_session_id: Option<String>,
    upstream_cost: Option<UpstreamCost>,
    upstream_correlation_id: Option<String>,
    credential_guard: CredentialExposureGuard,
) {
    let mut usage = usage;
    if response_numeric_material_contains_credential(
        &credential_guard,
        &usage,
        upstream_cost.as_ref(),
    ) {
        log::warn!("Usage event omitted because response repeated protected credential material");
        return;
    }
    usage.message_id = credential_guard.redact_option(usage.message_id);
    let model = credential_guard.redact_or(model, "unknown").to_string();
    let request_model = credential_guard
        .redact_or(request_model, "unknown")
        .to_string();
    let outbound_model = credential_guard
        .redact_or(outbound_model, "unknown")
        .to_string();
    let stable_session_id = credential_guard.redact_option(stable_session_id);
    let legacy_session_id = credential_guard.redact_option(legacy_session_id);
    let upstream_correlation_id = credential_guard.redact_option(upstream_correlation_id);
    let logger = UsageLogger::new(&state.db);
    let (multiplier, pricing_model_source) = logger
        .resolve_binding_pricing_config(&pricing_override, app_type)
        .await;
    let multiplier = credential_safe_cost_multiplier(&credential_guard, multiplier);
    let pricing_model = if pricing_model_source == PRICING_SOURCE_REQUEST {
        outbound_model.clone()
    } else {
        model.clone()
    };
    let event_id = usage.dedup_request_id();
    let upstream_correlation_id = upstream_correlation_id.or_else(|| usage.message_id.clone());
    let input = UsageIngestionInput {
        event_id: event_id.clone(),
        source: TokenSource::Proxy,
        provider_id: usage_provider_id.to_string(),
        agent_module_id: agent_module_id.to_string(),
        frozen_provider_context: Some(frozen_provider_context),
        occurred_at: chrono::Utc::now().timestamp(),
        model,
        usage,
        upstream_cost,
        request_id: None,
        session_id: stable_session_id,
        upstream_correlation_id,
        legacy: Some(LegacyLogInput {
            request_id: event_id.clone(),
            provider_id: legacy_provider_id.to_string(),
            app_type: app_type.to_string(),
            request_model,
            pricing_model,
            latency_ms,
            first_token_ms,
            status_code,
            error_message: None,
            session_id: legacy_session_id,
            provider_type: None,
            is_streaming,
            cost_multiplier: multiplier,
        }),
    };

    if let Err(error) = logger.ingest_with_credential_guard(&input, &credential_guard) {
        report_ingestion_failure(usage_provider_id, &event_id, &error);
    }
}

fn response_numeric_material_contains_credential(
    credential_guard: &CredentialExposureGuard,
    usage: &TokenUsage,
    upstream_cost: Option<&UpstreamCost>,
) -> bool {
    [
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_creation_tokens,
    ]
    .into_iter()
    .any(|value| credential_guard.contains(&value.to_string()))
        || upstream_cost.is_some_and(|cost| {
            [
                cost.input_cost.as_ref(),
                cost.output_cost.as_ref(),
                cost.cache_read_cost.as_ref(),
                cost.cache_creation_cost.as_ref(),
                cost.total_cost.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| credential_guard.contains(&value.to_string()))
        })
}

fn credential_safe_cost_multiplier(
    credential_guard: &CredentialExposureGuard,
    multiplier: rust_decimal::Decimal,
) -> rust_decimal::Decimal {
    if !credential_guard.contains(&multiplier.to_string()) {
        return multiplier;
    }

    // A low-entropy numeric key can collide with otherwise legitimate pricing
    // metadata. Pick a deterministic neutral fallback whose serialized value
    // does not reproduce that key; never persist the colliding value merely
    // because it parsed as a Decimal.
    [
        rust_decimal::Decimal::ONE,
        rust_decimal::Decimal::ZERO,
        rust_decimal::Decimal::from(2_u32),
        rust_decimal::Decimal::NEGATIVE_ONE,
    ]
    .into_iter()
    .find(|candidate| !credential_guard.contains(&candidate.to_string()))
    .unwrap_or(rust_decimal::Decimal::ZERO)
}

pub(crate) fn report_ingestion_failure(
    provider_id: &str,
    request_id: &str,
    error: &crate::error::AppError,
) {
    log::warn!("[USG-001] 记录使用量失败: provider={provider_id}, request={request_id}: {error}");
    crate::usage_events::notify_ingestion_error(provider_id, request_id);
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)] // v12 compatibility tests; request paths use ingest_usage_internal.
async fn log_usage_internal(
    state: &ProxyState,
    provider_id: &str,
    app_type: &str,
    model: &str,
    request_model: &str,
    outbound_model: &str,
    usage: TokenUsage,
    latency_ms: u64,
    first_token_ms: Option<u64>,
    is_streaming: bool,
    status_code: u16,
    session_id: Option<String>,
) {
    let logger = UsageLogger::new(&state.db);
    let (multiplier, pricing_model_source) =
        logger.resolve_pricing_config(provider_id, app_type).await;
    let pricing_model = if pricing_model_source == PRICING_SOURCE_REQUEST {
        outbound_model
    } else {
        model
    };

    let request_id = usage.dedup_request_id();

    log::debug!(
        "[{app_type}] 记录请求日志: id={request_id}, provider={provider_id}, model={model}, streaming={is_streaming}, status={status_code}, latency_ms={latency_ms}, first_token_ms={first_token_ms:?}, session={}, input={}, output={}, cache_read={}, cache_creation={}",
        session_id.as_deref().unwrap_or("none"),
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_creation_tokens
    );

    if let Err(e) = logger.log_with_calculation(
        request_id,
        provider_id.to_string(),
        app_type.to_string(),
        model.to_string(),
        request_model.to_string(),
        pricing_model.to_string(),
        usage,
        multiplier,
        latency_ms,
        first_token_ms,
        status_code,
        session_id,
        None, // provider_type
        is_streaming,
    ) {
        log::warn!("[USG-001] 记录使用量失败: {e}");
    }
}

/// 创建带日志记录和超时控制的透传流
pub fn create_logged_passthrough_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    tag: &'static str,
    usage_collector: Option<SseUsageCollector>,
    timeout_config: StreamingTimeoutConfig,
    connection_guard: Option<ActiveConnectionGuard>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let _conn_guard = connection_guard;
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut collector = usage_collector;
        let mut finish_guard = collector.clone().map(SseUsageFinishGuard::new);
        let inspect_sse_events =
            collector.is_some() || log::log_enabled!(log::Level::Debug);
        let mut is_first_chunk = true;

        // 超时配置
        let first_byte_timeout = if timeout_config.first_byte_timeout > 0 {
            Some(Duration::from_secs(timeout_config.first_byte_timeout))
        } else {
            None
        };
        let idle_timeout = if timeout_config.idle_timeout > 0 {
            Some(Duration::from_secs(timeout_config.idle_timeout))
        } else {
            None
        };

        tokio::pin!(stream);

        loop {
            // 选择超时时间：首字节超时或静默期超时
            let timeout_duration = if is_first_chunk {
                first_byte_timeout
            } else {
                idle_timeout
            };

            let chunk_result = match timeout_duration {
                Some(duration) => {
                    match tokio::time::timeout(duration, stream.next()).await {
                        Ok(Some(chunk)) => Some(chunk),
                        Ok(None) => None, // 流结束
                        Err(_) => {
                            // 超时
                            let timeout_type = if is_first_chunk { "首字节" } else { "静默期" };
                            log::error!("[{tag}] 流式响应{}超时 ({}秒)", timeout_type, duration.as_secs());
                            yield Err(std::io::Error::other(format!("流式响应{timeout_type}超时")));
                            break;
                        }
                    }
                }
                None => stream.next().await, // 无超时限制
            };

            match chunk_result {
                Some(Ok(bytes)) => {
                    if is_first_chunk {
                        log::debug!(
                            "[{tag}] 已接收上游流式首包: bytes={}",
                            bytes.len()
                        );
                    }
                    is_first_chunk = false;
                    if inspect_sse_events {
                        crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                        // 尝试解析并记录完整的 SSE 事件
                        while let Some(event_text) = take_sse_block(&mut buffer) {
                            if !event_text.trim().is_empty() {
                                // 提取 data 部分；只有 usage collector 存在时才解析 JSON。
                                for line in event_text.split(['\r', '\n']) {
                                    if let Some(data) = strip_sse_field(line, "data") {
                                        if data.trim() != "[DONE]" {
                                            let collected = match &collector {
                                                Some(c) if c.should_collect(data) => {
                                                    match serde_json::from_str::<Value>(data) {
                                                        Ok(json_value) => {
                                                            c.push(json_value).await;
                                                            true
                                                        }
                                                        Err(_) => false,
                                                    }
                                                }
                                                _ => false,
                                            };
                                            if collected {
                                                log::debug!("[{tag}] <<< SSE usage 事件已收集");
                                            } else {
                                                log::debug!("[{tag}] <<< SSE 数据事件");
                                            }
                                        } else {
                                            log::debug!("[{tag}] <<< SSE: [DONE]");
                                        }
                                    }
                                }
                            }
                        }
                    }

                    yield Ok(bytes);
                }
                Some(Err(_)) => {
                    log::error!("[{tag}] 流错误；细节已省略");
                    yield Err(std::io::Error::other("upstream stream failed"));
                    break;
                }
                None => {
                    // 流正常结束
                    break;
                }
            }
        }

        if let Some(c) = collector.take() {
            c.finish().await;
        }
        if let Some(guard) = &mut finish_guard {
            guard.disarm();
        }
    }
}

fn format_headers(headers: &HeaderMap) -> String {
    format!("<{} headers>", headers.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{
        unavailable_credential_store, BindingCredentialService, CredentialStore,
        CredentialStoreError, SecretString,
    };
    use crate::database::Database;
    use crate::error::AppError;
    use crate::provider::Provider;
    use crate::provider::ProviderMeta;
    use crate::proxy::provider_router::ProviderRouter;
    use crate::proxy::providers::{
        codex_chat_history::CodexChatHistoryStore, gemini_shadow::GeminiShadowStore,
    };
    use crate::proxy::types::{ProxyConfig, ProxyStatus};
    use crate::usage::domain::{
        AgentProviderBindingInput, BillingKind, TokenSource, UsageProviderInput,
    };
    use axum::http::StatusCode;
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn frozen_provider_context(product_group_id: &str) -> FrozenUsageProviderContext {
        FrozenUsageProviderContext {
            product_group_id: product_group_id.to_string(),
            route_app_type: product_group_id.to_string(),
        }
    }

    #[test]
    fn credential_bearing_success_body_is_rejected_before_parsing_or_egress() {
        const SECRET: &str = "protected-response-echo-key";
        let guard = CredentialExposureGuard::from_secret(SECRET.as_bytes());
        let body = format!(r#"{{"id":"msg-{SECRET}","model":"safe"}}"#);

        assert!(matches!(
            reject_credential_bearing_response_body(body.as_bytes(), &guard),
            Err(ProxyError::UpstreamResponseRejected)
        ));
    }

    #[tokio::test]
    async fn credential_bearing_stream_stops_before_chunk_completing_the_key() {
        use futures::StreamExt;

        const SECRET: &str = "protected-stream-echo-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(b"data: prefix-protected-stream")),
            Ok(Bytes::from_static(b"-echo-key-suffix\n\n")),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;

        assert_eq!(output.len(), 1);
        assert!(output[0].is_err());
        let visible = output
            .into_iter()
            .filter_map(Result::ok)
            .flat_map(|bytes| bytes.to_vec())
            .collect::<Vec<_>>();
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_rejects_native_claude_text_deltas() {
        use futures::StreamExt;

        const SECRET: &str = "protected-native-claude-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(
                b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"protected-native-\"}}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"claude-key\"}}\n\n",
            )),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_rejects_native_codex_output_deltas() {
        use futures::StreamExt;

        const SECRET: &str = "protected-native-codex-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"protected-native-\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"codex-key\"}\n\n",
            )),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_supports_cr_only_event_boundaries() {
        use futures::StreamExt;

        const SECRET: &str = "protected-cr-boundary-key";
        let stream = futures::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"protected-cr-\"}\r\rdata: {\"type\":\"response.output_text.delta\",\"delta\":\"boundary-key\"}\r\r",
        ))]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_isolates_interleaved_content_indices() {
        use futures::StreamExt;

        const SECRET: &str = "protected-interleave-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(
                b"data: {\"index\":0,\"delta\":{\"text\":\"protected-interleave-\"}}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"index\":1,\"delta\":{\"text\":\"safe\"}}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"index\":0,\"delta\":{\"text\":\"key\"}}\n\n",
            )),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_tracks_reasoning_despite_sibling_delta_scalars() {
        use futures::StreamExt;

        const SECRET: &str = "protected-reasoning-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(
                b"data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning\":\"protected-reasoning-\"}}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning\":\"key\"}}]}\n\n",
            )),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_retains_split_encoding_introducers() {
        use futures::StreamExt;

        const SECRET: &str = "secret-encoded-boundary";
        let encoded = SECRET
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        let percent_stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(b"data: {\"delta\":\"%\"}\n\n")),
            Ok(Bytes::from(format!(
                "data: {{\"delta\":\"{}\"}}\n\n",
                &encoded[1..]
            ))),
        ]);
        let percent_output = guard_credential_response_stream(
            percent_stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        assert!(percent_output.iter().any(Result::is_err));
        assert!(percent_output.iter().all(|item| item.is_err()));

        let escaped = SECRET
            .bytes()
            .map(|byte| format!(r"\u{byte:04x}"))
            .collect::<String>();
        let json_stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(b"data: {\"delta\":\"\\\\\"}\n\n")),
            Ok(Bytes::from(format!(
                "data: {{\"delta\":{}}}\n\n",
                serde_json::to_string(&escaped[1..]).unwrap()
            ))),
        ]);
        let json_output = guard_credential_response_stream(
            json_stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        assert!(json_output.iter().any(Result::is_err));
        assert!(json_output.iter().all(|item| item.is_err()));
    }

    #[tokio::test]
    async fn safe_final_single_byte_prefix_is_released_at_eof() {
        use futures::StreamExt;

        let event = Bytes::from_static(b"data: {\"delta\":\"s\"}\n\n");
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![Ok::<_, std::io::Error>(event.clone())]),
            CredentialExposureGuard::from_secret(b"sk-safe-final-prefix-key"),
        )
        .collect::<Vec<_>>()
        .await;

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].as_ref().unwrap(), &event);
    }

    #[tokio::test]
    async fn large_safe_semantic_value_ending_in_a_prefix_is_released_at_eof() {
        use futures::StreamExt;

        let event = Bytes::from(format!(
            "data: {{\"delta\":{}}}\n\n",
            serde_json::to_string(&format!("{}s", "x".repeat(300 * 1024))).unwrap()
        ));
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![Ok::<_, std::io::Error>(event.clone())]),
            CredentialExposureGuard::from_secret(b"sk-safe-large-final-prefix-key"),
        )
        .collect::<Vec<_>>()
        .await;

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].as_ref().unwrap(), &event);
    }

    #[tokio::test]
    async fn semantic_sse_guard_scans_the_bom_prefixed_first_event() {
        use futures::StreamExt;

        const SECRET: &str = "protected-bom-key";
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Bytes::from_static(
                b"\xEF\xBB\xBFdata: {\"index\":0,\"delta\":{\"text\":\"protected-bom-\"}}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"index\":0,\"delta\":{\"text\":\"key\"}}\n\n",
            )),
        ]);
        let output = guard_credential_response_stream(
            stream,
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn semantic_sse_guard_never_releases_a_prefix_across_large_safe_padding() {
        use futures::StreamExt;

        const SECRET: &str = "protected-padded-semantic-key";
        let prefix = Bytes::from_static(
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"protected-padded-\"}\n\n",
        );
        let padding = Bytes::from(format!(
            "event: ping\ndata: {{\"type\":\"ping\",\"padding\":\"{}\"}}\n\n",
            "x".repeat(16 * 1024)
        ));
        let suffix = Bytes::from_static(
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"semantic-key\"}\n\n",
        );
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![
                Ok::<_, std::io::Error>(prefix),
                Ok(padding),
                Ok(suffix),
            ]),
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn deeply_percent_encoded_sse_is_rejected_before_any_prefix_is_released() {
        use futures::StreamExt;

        const SECRET: &str = "protected-five-layer-key";
        let mut encoded = SECRET.as_bytes().to_vec();
        for _ in 0..5 {
            encoded = encoded
                .iter()
                .flat_map(|byte| format!("%{byte:02X}").into_bytes())
                .collect();
        }
        let mut first = b"data: {\"delta\":\"".to_vec();
        first.extend_from_slice(&encoded[..encoded.len() - 1]);
        let mut second = vec![*encoded.last().unwrap()];
        second.extend_from_slice(b"\"}\n\n");
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![
                Ok::<_, std::io::Error>(Bytes::from(first)),
                Ok(Bytes::from(second)),
            ]),
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn alternating_encoded_sse_is_rejected_before_egress() {
        use futures::StreamExt;

        const SECRET: &str = "protected-alternating-key";
        let inner_percent = SECRET
            .bytes()
            .flat_map(|byte| format!("%{byte:02X}").into_bytes())
            .collect::<Vec<_>>();
        let json_escaped = inner_percent
            .iter()
            .flat_map(|byte| format!(r"\u{byte:04x}").into_bytes())
            .collect::<Vec<_>>();
        let outer_percent = json_escaped
            .iter()
            .flat_map(|byte| format!("%{byte:02X}").into_bytes())
            .collect::<Vec<_>>();
        let mut event = b"data: {\"delta\":\"".to_vec();
        event.extend_from_slice(&outer_percent);
        event.extend_from_slice(b"\"}\n\n");
        let split = event.len() / 2;
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![
                Ok::<_, std::io::Error>(Bytes::copy_from_slice(&event[..split])),
                Ok(Bytes::copy_from_slice(&event[split..])),
            ]),
            CredentialExposureGuard::from_secret(SECRET.as_bytes()),
        )
        .collect::<Vec<_>>()
        .await;
        let visible = output
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .flat_map(|bytes| bytes.iter().copied())
            .collect::<Vec<_>>();

        assert!(output.iter().any(Result::is_err));
        assert!(visible.is_empty());
    }

    #[tokio::test]
    async fn unterminated_sse_event_is_bounded_and_fails_closed() {
        use futures::StreamExt;

        let mut oversized = b"data: {\"delta\":\"".to_vec();
        oversized.resize(MAX_CREDENTIAL_GUARD_PENDING_BYTES + 1, b'x');
        let output = guard_credential_response_stream(
            futures::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(oversized))]),
            CredentialExposureGuard::from_secret(b"protected-bounded-event-key"),
        )
        .collect::<Vec<_>>()
        .await;

        assert_eq!(output.len(), 1);
        assert!(output[0].is_err());
    }

    #[test]
    fn semantic_non_stream_guard_rejects_split_content_blocks() {
        const SECRET: &str = "protected-content-block-key";
        let body = serde_json::to_vec(&json!({
            "id": "safe-response",
            "content": [
                { "type": "text", "text": "protected-content-" },
                { "type": "text", "text": "block-key" }
            ]
        }))
        .unwrap();

        assert!(matches!(
            reject_credential_bearing_response_body(
                &body,
                &CredentialExposureGuard::from_secret(SECRET.as_bytes()),
            ),
            Err(ProxyError::UpstreamResponseRejected)
        ));
    }

    #[tokio::test]
    async fn ingestion_failure_does_not_change_successful_upstream_response() -> Result<(), AppError>
    {
        use http_body_util::BodyExt;

        let db = Arc::new(Database::memory()?);
        let settings = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://upstream.example",
                "ANTHROPIC_AUTH_TOKEN": "secret"
            }
        });
        db.save_provider(
            "claude",
            &Provider::with_id(
                "legacy-provider".to_string(),
                "Legacy".to_string(),
                settings.clone(),
                None,
            ),
        )?;
        db.save_usage_provider(&UsageProviderInput {
            id: "global-provider".to_string(),
            name: "Global".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("claude".to_string()),
            route_config: Some(settings),
            quota_config: None,
            enabled: true,
        })?;
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers
                 SET legacy_app_type='claude', legacy_provider_id='legacy-provider'
                 WHERE id='global-provider'",
                [],
            )?;
        }
        db.set_route_binding("claude", "global-provider")?;
        let binding = db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: "claude-code".to_string(),
            provider_id: "global-provider".to_string(),
            enabled: false,
        })?;
        let expected_binding_id = binding.id.clone();
        let credential_service = Arc::new(BindingCredentialService::new(
            db.clone(),
            Arc::new(TestCredentialStore::default()),
        ));
        let binding = credential_service
            .set_binding_api_key(
                &binding.id,
                binding.credential_version,
                SecretString::new("local-binding-key".to_string()),
            )
            .await?;
        db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: Some(binding.id),
            agent_module_id: binding.agent_module_id,
            provider_id: binding.provider_id,
            enabled: true,
        })?;
        let state = build_state_with_credential_service(db.clone(), credential_service);
        let ctx = RequestContext::new(
            &state,
            &serde_json::json!({"model": "claude-sonnet"}),
            &HeaderMap::new(),
            SecretString::new("local-binding-key".to_string()),
            crate::app_config::AppType::Claude,
            "Claude",
            "claude",
        )
        .await
        .map_err(|error| AppError::Message(error.to_string()))?;
        assert_eq!(ctx.binding_id, expected_binding_id);
        assert_eq!(ctx.agent_module_id, "claude-code");
        assert_eq!(ctx.usage_provider_id, "global-provider");
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_usage_ingestion
                 BEFORE INSERT ON usage_events
                 BEGIN
                   SELECT RAISE(ABORT, 'forced ingestion failure');
                 END;",
            )?;
        }

        let original = serde_json::json!({
            "id": "resp-success",
            "model": "claude-sonnet",
            "usage": {"input_tokens": 3, "output_tokens": 4, "cost": "0.01"},
            "content": []
        });
        let original_bytes = Bytes::from(serde_json::to_vec(&original).unwrap());
        let response = super::handle_non_streaming(
            ProxyResponse::buffered(StatusCode::OK, HeaderMap::new(), original_bytes.clone()),
            &ctx,
            &state,
            &crate::proxy::handler_config::CLAUDE_PARSER_CONFIG,
            None,
        )
        .await
        .map_err(|error| AppError::Message(error.to_string()))?;

        assert_eq!(response.status(), StatusCode::OK);
        let returned = response
            .into_body()
            .collect()
            .await
            .map_err(|error| AppError::Message(error.to_string()))?
            .to_bytes();
        assert_eq!(returned, original_bytes);
        tokio::time::sleep(Duration::from_millis(25)).await;
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| row
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    }

    #[test]
    fn non_streaming_response_id_is_the_upstream_correlation_id() {
        let body = serde_json::json!({"id": "resp-exact", "usage": {}});
        assert_eq!(
            upstream_correlation_id_from_body(&body).as_deref(),
            Some("resp-exact")
        );
    }

    #[test]
    fn terminal_sse_response_id_is_the_upstream_correlation_id() {
        let events = vec![
            serde_json::json!({"type": "response.created", "response": {"id": "resp-old"}}),
            serde_json::json!({"type": "response.completed", "response": {"id": "resp-final"}}),
        ];
        assert_eq!(
            upstream_correlation_id_from_events(&events).as_deref(),
            Some("resp-final")
        );
    }

    #[test]
    fn generated_session_id_is_not_stable_cross_source_evidence() {
        assert_eq!(stable_session_id("generated-uuid", false), None);
        assert_eq!(
            stable_session_id("client-session", true).as_deref(),
            Some("client-session")
        );
    }

    #[test]
    fn invalid_explicit_cost_is_not_eligible_for_estimated_ingestion() {
        let invalid = extract_upstream_cost(&serde_json::json!({
            "usage": {"cost": "-0.01"}
        }));
        assert!(validated_upstream_cost(invalid, "provider", "request").is_none());

        let absent = extract_upstream_cost(&serde_json::json!({
            "usage": {"input_tokens": 1, "output_tokens": 2}
        }));
        assert!(matches!(
            validated_upstream_cost(absent, "provider", "request"),
            Some(None)
        ));
    }

    #[tokio::test]
    async fn raw_sse_tee_preserves_bytes_cost_and_exact_upstream_id() -> Result<(), std::io::Error>
    {
        let first = Bytes::from_static(b"data: {\"id\":\"chatcmpl-raw\",\"choices\":[]}\n\n");
        let second = Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-raw\",\"usage\":{\"total_cost\":\"0.42\"}}\n\n",
        );
        let expected = [first.clone(), second.clone()].concat();
        let metadata = Arc::new(StdMutex::new(RawSseUsageMetadata::default()));
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(first),
            Ok::<_, std::io::Error>(second),
        ]);

        let captured =
            capture_raw_sse_usage_metadata(stream, metadata.clone(), "global-provider".to_string())
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
                .concat();

        assert_eq!(captured, expected);
        let (cost, correlation_id) =
            validated_raw_sse_usage_metadata(&metadata).expect("valid raw metadata");
        assert_eq!(correlation_id.as_deref(), Some("chatcmpl-raw"));
        assert_eq!(
            cost.and_then(|cost| cost.total_cost)
                .map(|value| value.to_string())
                .as_deref(),
            Some("0.42")
        );
        Ok(())
    }

    #[test]
    fn test_strip_sse_field_accepts_optional_space() {
        assert_eq!(
            super::strip_sse_field("data: {\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("data:{\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("event: message_start", "event"),
            Some("message_start")
        );
        assert_eq!(
            super::strip_sse_field("event:message_start", "event"),
            Some("message_start")
        );
        assert_eq!(super::strip_sse_field("id:1", "data"), None);
    }

    #[test]
    fn formatted_response_headers_log_names_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer response-secret"),
        );
        headers.insert(
            "x-api-key",
            axum::http::HeaderValue::from_static("response-secret"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "x-debug",
            axum::http::HeaderValue::from_static("upstream-reflected-secret"),
        );

        let formatted = format_headers(&headers);
        assert!(!formatted.contains("response-secret"));
        assert!(!formatted.contains("upstream-reflected-secret"));
        assert!(!formatted.contains("application/json"));
        assert!(!formatted.contains("authorization"));
        assert!(!formatted.contains("x-api-key"));
        assert!(!formatted.contains("content-type"));
        assert!(!formatted.contains("x-debug"));
        assert_eq!(formatted, "<4 headers>");
    }

    #[test]
    fn protected_binding_key_is_removed_from_upstream_response_headers() {
        let guard = CredentialExposureGuard::from_secret(b"response-binding/key");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-upstream-debug",
            axum::http::HeaderValue::from_static("response-binding%2Fkey"),
        );
        headers.insert(
            "x-safe",
            axum::http::HeaderValue::from_static("safe-response-value"),
        );

        strip_credential_bearing_response_headers(&mut headers, &guard);

        assert!(!headers.contains_key("x-upstream-debug"));
        assert_eq!(
            headers.get("x-safe"),
            Some(&axum::http::HeaderValue::from_static("safe-response-value"))
        );
    }

    #[test]
    fn test_strip_hop_by_hop_response_headers_removes_standard_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("keep-alive"),
            axum::http::HeaderValue::from_static("timeout=5"),
        );
        headers.insert(
            axum::http::header::TRANSFER_ENCODING,
            axum::http::HeaderValue::from_static("chunked"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("proxy-connection"),
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            axum::http::header::CONTENT_LENGTH,
            axum::http::HeaderValue::from_static("12"),
        );

        strip_hop_by_hop_response_headers(&mut headers);

        assert!(!headers.contains_key(axum::http::header::CONNECTION));
        assert!(!headers.contains_key("keep-alive"));
        assert!(!headers.contains_key(axum::http::header::TRANSFER_ENCODING));
        assert!(!headers.contains_key("proxy-connection"));
        assert_eq!(
            headers.get(axum::http::header::CONTENT_TYPE),
            Some(&axum::http::HeaderValue::from_static("application/json"))
        );
        assert_eq!(
            headers.get(axum::http::header::CONTENT_LENGTH),
            Some(&axum::http::HeaderValue::from_static("12"))
        );
    }

    #[test]
    fn test_strip_hop_by_hop_response_headers_removes_connection_listed_extensions() {
        let mut headers = HeaderMap::new();
        headers.append(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("x-trace-hop, x-debug-hop"),
        );
        headers.append(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("upgrade"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("x-trace-hop"),
            axum::http::HeaderValue::from_static("trace"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("x-debug-hop"),
            axum::http::HeaderValue::from_static("debug"),
        );
        headers.insert(
            axum::http::header::UPGRADE,
            axum::http::HeaderValue::from_static("websocket"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );

        strip_hop_by_hop_response_headers(&mut headers);

        assert!(!headers.contains_key(axum::http::header::CONNECTION));
        assert!(!headers.contains_key("x-trace-hop"));
        assert!(!headers.contains_key("x-debug-hop"));
        assert!(!headers.contains_key(axum::http::header::UPGRADE));
        assert_eq!(
            headers.get(axum::http::header::CONTENT_TYPE),
            Some(&axum::http::HeaderValue::from_static("text/event-stream"))
        );
    }

    #[derive(Default)]
    struct TestCredentialStore {
        items: StdMutex<HashMap<String, Vec<u8>>>,
    }

    impl CredentialStore for TestCredentialStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
            self.items
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(slot.to_string(), secret.to_vec());
            Ok(())
        }

        fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
            Ok(self
                .items
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(slot)
                .cloned())
        }

        fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
            self.items
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(slot);
            Ok(())
        }
    }

    fn build_state(db: Arc<Database>) -> ProxyState {
        let credential_service = Arc::new(BindingCredentialService::new(
            db.clone(),
            unavailable_credential_store(),
        ));
        build_state_with_credential_service(db, credential_service)
    }

    fn build_state_with_credential_service(
        db: Arc<Database>,
        binding_credential_service: Arc<BindingCredentialService>,
    ) -> ProxyState {
        ProxyState {
            db: db.clone(),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
            provider_router: Arc::new(ProviderRouter::new(db.clone())),
            binding_credential_service,
            gemini_shadow: Arc::new(GeminiShadowStore::default()),
            codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
            app_handle: None,
        }
    }

    fn seed_pricing(db: &Database) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["resp-model", "Resp Model", "1.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["req-model", "Req Model", "2.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    #[tokio::test]
    async fn proxy_ingestion_persists_frozen_binding_and_provider_context() -> Result<(), AppError>
    {
        let db = Arc::new(Database::memory()?);
        db.save_usage_provider(&UsageProviderInput {
            id: "global-provider".to_string(),
            name: "Global Provider".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "codex".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("codex".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        crate::database::lock_conn!(db.conn).execute(
            "UPDATE usage_providers
             SET product_group_id = 'mutated-group', route_app_type = 'claude'
             WHERE id = 'global-provider'",
            [],
        )?;
        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 3,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: Some("response-owned".to_string()),
        };

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            "gpt-5",
            "gpt-5",
            "gpt-5",
            usage,
            10,
            None,
            false,
            200,
            None,
            None,
            None,
            Some("response-owned".to_string()),
            CredentialExposureGuard::from_secret(b"nonmatching-binding-key"),
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (agent_module_id, provider_id, product_group_id): (Option<String>, String, String) =
            conn.query_row(
                "SELECT agent_module_id, provider_id, product_group_id
                 FROM usage_events WHERE event_id = 'session:response-owned'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        assert_eq!(agent_module_id.as_deref(), Some("codex"));
        assert_eq!(provider_id, "global-provider");
        assert_eq!(product_group_id, "codex");
        Ok(())
    }

    #[tokio::test]
    async fn protected_binding_key_is_removed_from_success_persistence() -> Result<(), AppError> {
        const PROTECTED_KEY: &str = "protected-binding-key-sentinel";

        let db = Arc::new(Database::memory()?);
        db.save_usage_provider(&UsageProviderInput {
            id: "global-provider".to_string(),
            name: "Global Provider".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "codex".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("codex".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        let state = build_state(db.clone());
        let echoed = format!("upstream-echo-{PROTECTED_KEY}-tail");
        let usage = TokenUsage {
            input_tokens: 3,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: Some(echoed.clone()),
            message_id: Some(echoed.clone()),
        };

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            &echoed,
            &echoed,
            &echoed,
            usage,
            10,
            None,
            false,
            200,
            Some(echoed.clone()),
            Some(echoed.clone()),
            None,
            Some(echoed.clone()),
            CredentialExposureGuard::from_secret(PROTECTED_KEY.as_bytes()),
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let canonical: (String, String, String, String, String) = conn
            .query_row(
                "SELECT event_id, model, COALESCE(session_id, ''),
                        COALESCE(upstream_correlation_id, ''),
                        COALESCE(legacy_request_id, '')
                 FROM usage_events",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        let legacy: (String, String, String, String, String) = conn
            .query_row(
                "SELECT request_id, model, COALESCE(request_model, ''),
                        COALESCE(pricing_model, ''), COALESCE(session_id, '')
                 FROM proxy_request_logs",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|error| AppError::Database(error.to_string()))?;

        let persisted = format!("{canonical:?}{legacy:?}");
        assert!(!persisted.contains(PROTECTED_KEY));
        assert_eq!(canonical.1, "unknown");
        assert_eq!(canonical.2, "");
        assert_eq!(canonical.3, "");
        assert_eq!(legacy.1, "unknown");
        assert_eq!(legacy.2, "unknown");
        assert_eq!(legacy.3, "unknown");
        assert_eq!(legacy.4, "");
        Ok(())
    }

    #[tokio::test]
    async fn numeric_binding_key_cannot_reenter_logs_as_legacy_cost_multiplier(
    ) -> Result<(), AppError> {
        const PROTECTED_KEY: &str = "927451.3819";

        let db = Arc::new(Database::memory()?);
        db.save_usage_provider(&UsageProviderInput {
            id: "global-provider".to_string(),
            name: "Global Provider".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "codex".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("codex".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        insert_provider(
            &db,
            "legacy-provider",
            "codex",
            ProviderMeta {
                cost_multiplier: Some(PROTECTED_KEY.to_string()),
                ..ProviderMeta::default()
            },
        )?;
        let occurrences_before = db.export_sql_string()?.matches(PROTECTED_KEY).count();
        let state = build_state(db.clone());

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            "gpt-5",
            "gpt-5",
            "gpt-5",
            TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: Some("numeric-multiplier-event".to_string()),
            },
            10,
            None,
            false,
            200,
            None,
            None,
            None,
            None,
            CredentialExposureGuard::from_secret(PROTECTED_KEY.as_bytes()),
        )
        .await;

        let cost_multiplier: String = crate::database::lock_conn!(db.conn)
            .query_row(
                "SELECT cost_multiplier FROM proxy_request_logs",
                [],
                |row| row.get(0),
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        assert_ne!(cost_multiplier, PROTECTED_KEY);
        assert_eq!(
            db.export_sql_string()?.matches(PROTECTED_KEY).count(),
            occurrences_before,
            "success ingestion duplicated the protected key into request logs"
        );
        Ok(())
    }

    #[tokio::test]
    async fn numeric_binding_key_cannot_reenter_usage_or_upstream_cost_columns(
    ) -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        db.save_usage_provider(&UsageProviderInput {
            id: "global-provider".to_string(),
            name: "Global Provider".to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "codex".to_string(),
            token_sources: vec![TokenSource::Proxy],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("codex".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        let state = build_state(db.clone());

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            "gpt-5",
            "gpt-5",
            "gpt-5",
            TokenUsage {
                input_tokens: 927_451,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: Some("numeric-token-event".to_string()),
            },
            10,
            None,
            false,
            200,
            None,
            None,
            None,
            None,
            CredentialExposureGuard::from_secret(b"927451"),
        )
        .await;

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            "gpt-5",
            "gpt-5",
            "gpt-5",
            TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: Some("numeric-cost-event".to_string()),
            },
            10,
            None,
            false,
            200,
            None,
            None,
            Some(UpstreamCost {
                input_cost: None,
                output_cost: None,
                cache_read_cost: None,
                cache_creation_cost: None,
                total_cost: Some(Decimal::from_str("927451.3819").unwrap()),
            }),
            None,
            CredentialExposureGuard::from_secret(b"927451.3819"),
        )
        .await;

        ingest_usage_internal(
            &state,
            "codex",
            "global-provider",
            frozen_provider_context("codex"),
            "legacy-provider",
            BindingPricingOverride::default(),
            "codex",
            "gpt-5",
            "gpt-5",
            "gpt-5",
            TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: Some("numeric-status-event".to_string()),
            },
            10,
            None,
            false,
            200,
            None,
            None,
            None,
            None,
            CredentialExposureGuard::from_secret(b"200"),
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let canonical_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))?;
        let legacy_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                row.get(0)
            })?;
        assert_eq!(canonical_count, 0);
        assert_eq!(legacy_count, 0);
        Ok(())
    }

    fn insert_provider(
        db: &Database,
        id: &str,
        app_type: &str,
        meta: ProviderMeta,
    ) -> Result<(), AppError> {
        let meta_json =
            serde_json::to_string(&meta).map_err(|e| AppError::Database(e.to_string()))?;
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, app_type, "Test Provider", "{}", meta_json],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    #[tokio::test]
    async fn test_log_usage_uses_provider_override_config() -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "claude";

        db.set_default_cost_multiplier(app_type, "1.5").await?;
        db.set_pricing_model_source(app_type, "response").await?;
        seed_pricing(&db)?;

        let meta = ProviderMeta {
            cost_multiplier: Some("2".to_string()),
            pricing_model_source: Some("request".to_string()),
            ..ProviderMeta::default()
        };
        insert_provider(&db, "provider-1", app_type, meta)?;

        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        log_usage_internal(
            &state,
            "provider-1",
            app_type,
            "resp-model",
            "req-model",
            "req-model",
            usage,
            10,
            None,
            false,
            200,
            None,
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (model, request_model, total_cost, cost_multiplier): (String, String, String, String) =
            conn.query_row(
                "SELECT model, request_model, total_cost_usd, cost_multiplier
                 FROM proxy_request_logs WHERE provider_id = ?1",
                ["provider-1"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        assert_eq!(model, "resp-model");
        assert_eq!(request_model, "req-model");
        assert_eq!(
            Decimal::from_str(&cost_multiplier).unwrap(),
            Decimal::from_str("2").unwrap()
        );
        assert_eq!(
            Decimal::from_str(&total_cost).unwrap(),
            Decimal::from_str("4").unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_request_pricing_mode_anchors_to_outbound_model() -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "claude";

        db.set_pricing_model_source(app_type, "request").await?;
        seed_pricing(&db)?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('outbound-model', 'Outbound Model', '4.0', '0')",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        insert_provider(&db, "provider-3", app_type, ProviderMeta::default())?;

        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        // 路由接管场景：客户端请求 req-model（$2/M），代理实际发出 outbound-model
        // （$4/M），上游回显 resp-model。「按请求计价」必须锚定实际发出的模型。
        log_usage_internal(
            &state,
            "provider-3",
            app_type,
            "resp-model",
            "req-model",
            "outbound-model",
            usage,
            10,
            None,
            false,
            200,
            None,
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (model, request_model, total_cost): (String, String, String) = conn
            .query_row(
                "SELECT model, request_model, total_cost_usd
                 FROM proxy_request_logs WHERE provider_id = ?1",
                ["provider-3"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        // model / request_model 列不受计价锚点影响
        assert_eq!(model, "resp-model");
        assert_eq!(request_model, "req-model");
        // 按 outbound-model（$4/M）计价，而不是 req-model（$2/M）或 resp-model（$1/M）
        assert_eq!(
            Decimal::from_str(&total_cost).unwrap(),
            Decimal::from_str("4").unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_claude_desktop_inherits_claude_global_defaults() -> Result<(), AppError> {
        use crate::proxy::usage::logger::UsageLogger;

        let db = Arc::new(Database::memory()?);

        // 全局计费配置只有 claude/codex/gemini 三行；claude-desktop 的
        // 全局默认必须继承 claude，而不是静默落回工厂默认（1 / response）
        db.set_default_cost_multiplier("claude", "1.5").await?;
        db.set_pricing_model_source("claude", "request").await?;

        let logger = UsageLogger::new(&db);
        let (multiplier, source) = logger
            .resolve_pricing_config("nonexistent-provider", "claude-desktop")
            .await;

        assert_eq!(multiplier, Decimal::from_str("1.5").unwrap());
        assert_eq!(source, "request");
        Ok(())
    }

    #[tokio::test]
    async fn test_log_usage_falls_back_to_global_defaults() -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "claude";

        db.set_default_cost_multiplier(app_type, "1.5").await?;
        db.set_pricing_model_source(app_type, "response").await?;
        seed_pricing(&db)?;

        let meta = ProviderMeta::default();
        insert_provider(&db, "provider-2", app_type, meta)?;

        let state = build_state(db.clone());
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        log_usage_internal(
            &state,
            "provider-2",
            app_type,
            "resp-model",
            "req-model",
            "req-model",
            usage,
            10,
            None,
            false,
            200,
            None,
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (total_cost, cost_multiplier): (String, String) = conn
            .query_row(
                "SELECT total_cost_usd, cost_multiplier
                 FROM proxy_request_logs WHERE provider_id = ?1",
                ["provider-2"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        assert_eq!(
            Decimal::from_str(&cost_multiplier).unwrap(),
            Decimal::from_str("1.5").unwrap()
        );
        assert_eq!(
            Decimal::from_str(&total_cost).unwrap(),
            Decimal::from_str("1.5").unwrap()
        );
        Ok(())
    }
}
