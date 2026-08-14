# T6:HTTP 转发层

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T3、T4、T5 全部完成并合并**。
> 可与 T7 并行(不碰同一个文件)。
>
> **这是整批任务里最容易做复杂的一个。先读完 §6「明确不要做的事」再动手。**

---

## 目标

在 loopback 上起一个 HTTP 服务,接住 Codex 发来的请求,按 T4 算出的候选队列依次
转发到上游,失败按 T5 的判定换下一家,并把结果记进 T3 的 `router_attempts`。

---

## 1. 用什么(已在依赖里,不要另找)

- 服务端:**`axum 0.8`**(已在 `Cargo.toml`)。不要用 actix、warp、rocket。
- 客户端:**`crate::http_client::get()`**(全局共享的 `reqwest::Client`,带连接池)。
  **不要 `reqwest::Client::new()`** —— 那样每次都新建连接池。
- 异步运行时:已有的 tokio。**不要自己建 Runtime**,用 `tauri::async_runtime::spawn`。

---

## 2. 新建文件

```
src-tauri/src/router/server.rs     监听、路由、转发
```

在 `src-tauri/src/router/mod.rs` 里 `pub mod server;`。

---

## 3. 对外接口(照写)

```rust
/// 启动本地路由服务。绑定失败返回 Err，由调用方决定怎么提示。
///
/// 必须在 app 启动流程的**早期**调用——先把端口开起来，再做其余初始化
/// (设计文档决定 5②)。
pub async fn start(db: Arc<Database>, port: u16) -> Result<(), AppError>;
```

监听地址**固定为 `127.0.0.1`**,绝不监听 `0.0.0.0`。这是本机服务,暴露到网络上是
安全问题。

---

## 4. 请求处理流程(照做,不要自由发挥)

Codex 会 POST 到 `{base_url}/responses`。

```
1. 收到请求，读出 body（JSON）
2. 从 body 里取 "model" 字段 → logical_model
3. db.list_model_routes(logical_model)  →  routes
4. 读当前拉黑状态 + 当前模式  →  blacklist, mode
5. router::decision::candidates_for(&routes, &blacklist, &mode, now_ms)  →  candidates
6. candidates 为空 → 返回 503 + 一句说明，记一条 outcome = "skipped"
7. 依次遍历 candidates：
     a. 把 body 里的 "model" 换成 candidate.upstream_model
     b. 转发到 provider 的 base_url
     c. 用 T5 的 classify() 判定结果
     d. 成功 → 把响应流式透传给客户端，记 attempt(success)，结束
     e. 失败且 try_next → 记 attempt(failed)，按 verdict 写拉黑，继续下一个
     f. 失败且 !try_next → 记 attempt(failed)，把错误透传给客户端，结束
8. 队列走完仍未成功 → 返回最后一次的错误
```

### 4.1 流式与「已经吐字」

**一旦向客户端写出了第一个字节,`already_streaming` 就是 `true`**,此后传给
`classify()` 时必须如实传,于是它不会再让你换下一家(决定 15)。

**必须流式透传,不要把整个响应读进内存再返回。** 用 `axum::body::Body::from_stream`
配合 reqwest 的 `bytes_stream()`。

### 4.2 请求体缓存与上限

要能重发给下一家,就得把请求体留在手里。**但必须有上限**:

```rust
const MAX_BUFFERED_BODY: usize = 4 * 1024 * 1024; // 4 MB
```

超过上限:**不缓存,直接流式转给第一个候选,并且不再有故障转移能力**(此时
`already_streaming` 逻辑同样适用)。设计文档 §6.2。

缓存**在第一个字节返回后即可丢弃** —— 那之后不可能再换家了。

### 4.3 认证

**router 自己不持有任何凭据。** 每个 provider 的凭据怎么取,由 provider 侧负责。
本任务里:把上游请求需要的认证头**由调用方注入**,你只负责转发。

**绝对不要**把收到的 `Authorization` 头原样转给上游 —— 那是客户端的凭据,不是上游的。

---

## 5. 拉黑状态存在哪

**内存里,不落库。** 用 `OnceLock<Mutex<Vec<Blacklisted>>>` 或等价物。

理由:拉黑是短时状态(60–600 秒),重启后重新试探是正确行为,落库反而要处理过期清理。
仓库里 `usage/claude_oauth.rs` 有现成的同类写法(`rate_limit_block`),照抄那个风格。

---

## 6. 明确不要做的事

- ❌ **不要做协议转换**(Responses ↔ Chat Completions)。v1 不做,设计文档 §2.3。
  遇到 `wire_api` 不一致的候选,**直接跳过它并记 `skipped`**
- ❌ **不要加任何定时器**。没有健康检查、没有心跳、没有定时清理。拉黑过期靠**读的时候
  判断 `until_ms > now`**,不要起后台任务去清(设计文档 §6.4 第 2 条)
- ❌ 不要监听 `0.0.0.0`
- ❌ 不要把整个响应读进内存
- ❌ 不要在这里写 UI、不要碰 `src/`
- ❌ 不要改 `config.toml`(那是 T7)
- ❌ 不要加新依赖

---

## 7. 崩溃隔离(硬要求)

**每个连接一个 task,panic 不能冒到进程级** —— router 是新代码在处理来自网络的输入,
它崩了不能把整个 app 带走(设计文档决定 5①)。

axum 默认每个请求一个 task;**额外要求**:处理函数里任何可能 panic 的地方
(索引越界、unwrap)都要改成返回错误。**代码里不允许出现 `.unwrap()` 和 `.expect()`**
(测试代码除外)。

---

## 8. 测试

转发层不好做纯单测,但下面这些**必须**有:

1. `model` 字段改写:给定一个 body 和一个 upstream_model,改写后的 body 里
   `model` 是新值,**其余字段一字未动**(用 `serde_json` 比对整棵树)
2. body 超过 4 MB 时走不缓存路径(用一个构造的大 body 测,不要真发网络)
3. `wire_api` 不匹配的候选被跳过,并记了 `skipped`
4. 队列为空时返回 503
5. 拉黑写入后,同一 (provider, 模型) 在冷却期内不再出现在候选里(与 T4 联测)

**不要为了测试去起真实的上游服务。** 把「发请求」这一步抽成一个可替换的函数,测试时
喂假的响应。

---

## 9. 完成的标准

- `start()` 能在 `127.0.0.1` 上起来,收请求、按队列转发、失败换下一家
- 流式透传,不整体缓存响应
- 请求体缓存有 4 MB 上限
- 无定时器、无 `unwrap`/`expect`(测试除外)
- 上面 5 条测试通过
- 六项检查全绿,`test result:` 行贴进报告
