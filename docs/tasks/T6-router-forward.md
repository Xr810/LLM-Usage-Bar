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

### 1.1 共享 client 有两个坑,都得绕(先读这条再写转发)

看 `src-tauri/src/http_client.rs:245`:

```rust
Client::builder()
    .timeout(Duration::from_secs(600))
    .connect_timeout(Duration::from_secs(30))
```

**坑一:那个 600 秒是整个请求的总时长上限,包含流式响应体。**
一次长对话的 SSE 流跑过 10 分钟就会被拦腰砍断,而且看起来像上游断流。
转发**必须**用 `RequestBuilder::timeout(...)` 覆盖成一个足够大的值(建议
**3600 秒**),或者在这条请求上关掉总超时。**不要去改全局 client 的配置** ——
那会影响 app 里其他所有请求。

**坑二:共享 client 没有 read_timeout,拿不到「首字节超时」这个信号。**
T5 的 `AttemptResult::FirstByteTimeout` 要你自己造:用
`tokio::time::timeout(Duration::from_secs(60), client.send()).await`
包住「发出去到拿到响应头」这一段,超时了就产出 `FirstByteTimeout`。
**首字节之后的流不要再套这个超时** —— 那是正常的长流。

---

## 2. 新建文件

```
src-tauri/src/router/server.rs     监听、路由、转发
```

在 `src-tauri/src/router/mod.rs` 里 `pub mod server;`。

---

## 3. 对外接口(照写)

```rust
/// 上游认证头的提供方。**router 自己不持有任何凭据**——它只在要发请求的那一刻
/// 问一次「这家的认证头是什么」，拿到就用，用完不留。
///
/// 返回的是要原样加到上游请求上的头（通常是一个 `Authorization`，
/// 但某些 provider 需要多个，所以是 Vec）。
/// 返回 `Ok(vec![])` 表示这家不需要认证头；返回 `Err` 表示凭据取不到，
/// 调用方应把这个候选当作失败（`RequestRejected`，不换下一家没意义——
/// 换一家是另一份凭据，所以这里**换**）。
pub trait UpstreamAuth: Send + Sync {
    fn headers_for(&self, provider_id: &str) -> Result<Vec<(String, String)>, AppError>;
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
) -> Result<(), AppError>;
```

监听地址**固定为 `127.0.0.1`**,绝不监听 `0.0.0.0`。这是本机服务,暴露到网络上是
安全问题。

### 3.1 要注册的路由,只有一条

```
POST /v1/responses
```

**`/v1` 这个前缀不是可选的**:T7 会把 Codex 的 `base_url` 写成
`http://127.0.0.1:<port>/v1`,Codex 再往后面接 `/responses`。少了前缀,
Codex 打过来就是 404,而且是本地 404,排查起来很费时间。

其余路径一律返回 404,**不要**做通配转发。

---

## 4. 请求处理流程(照做,不要自由发挥)

Codex 会 POST 到 `{base_url}/responses`,即本机的 `/v1/responses`(见 §3.1)。

```
1. 收到请求，读出 body（JSON）
2. 从 body 里取 "model" 字段 → logical_model
3. db.list_model_routes(logical_model)  →  routes
4. 读当前拉黑状态（内存，见 §5）+ 当前模式（见 §4.4）  →  blacklist, mode
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

**router 自己不持有任何凭据。** 每个 provider 的凭据怎么取,由 §3 那个
`UpstreamAuth` 负责,你在要发请求的那一刻调
`auth.headers_for(&candidate.provider_id)`,把返回的头加到上游请求上。

**绝对不要**把收到的 `Authorization` 头原样转给上游 —— 那是客户端的凭据,不是上游的。
转发前**必须显式剥掉**客户端来的这几个头:`authorization`、`cookie`、
`x-api-key`、`openai-organization`。**用白名单还是黑名单由你定,但报告里要写清楚
最终转出去的头有哪些** —— 这是本任务唯一的安全面。

`headers_for` 返回 `Err` 时:记一条 `failed` / `failure_kind = "auth_unavailable"`,
**继续下一个候选**(换一家是另一份凭据,值得试)。

### 4.3.1 T3 留下的一个缺口,由你补上

T3 的 `dao/router.rs` 里只有 `outcome_to_db`(枚举 → 字符串),**没有反方向**
—— `outcome_from_db` 被误放进了 `#[cfg(test)] mod tests` 里,生产代码取不到。

你要记 attempt,迟早要把行读回来。**把 `outcome_from_db` 从 tests 里提到
`dao/router.rs` 的生产代码区**(与 `wire_api_from_db` 并排),签名
`fn outcome_from_db(raw: &str) -> Result<AttemptOutcome, AppError>`,
非法值返回 `AppError` 不 panic;测试里那份删掉,改用生产的那个。

**这是本任务唯一授权你改 `dao/router.rs` 的地方**,别的一行不要动。

### 4.4 模式从哪读

```rust
db.get_setting("router.mode")?   //  "auto" | "manual:<provider_id>"
```

**读不到、是空串、或者格式不认识,一律当 `Auto`。** 不要报错、不要写回默认值。
理由:模式是用户的方向盘,拿不准的时候「自动」是唯一不会让他卡住的选择。

`manual:` 后面的 provider_id **不做存在性校验** —— T4 的 `candidates_for` 对
不存在的 provider 返回空 Vec,走第 6 步的 503,用户看得见。

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
6. **客户端来的 `Authorization` 不出现在转出去的头里**,而 `UpstreamAuth`
   返回的头出现了(§4.3 的安全面,必须有测试盯着)
7. `router.mode` 是 `"manual:packyapi"` 时只试那一家;是垃圾字符串
   (如 `"manual:"`、`"whatever"`)时**退回 auto**,不报错

**不要为了测试去起真实的上游服务。** 把「发请求」这一步抽成一个可替换的函数,测试时
喂假的响应;`UpstreamAuth` 同样用假实现。

---

## 9. 完成的标准

- `start()` 能在 `127.0.0.1` 上起来,只注册 `POST /v1/responses`,
  收请求、按队列转发、失败换下一家
- `UpstreamAuth` trait 已定义并在转发时调用;router 内部不出现任何凭据读取
- 流式透传,不整体缓存响应;总超时已覆盖成 3600 秒,首字节超时用
  `tokio::time::timeout` 单独造
- 请求体缓存有 4 MB 上限
- 无定时器、无 `unwrap`/`expect`(测试除外)
- 上面 7 条测试通过
- 报告里写明:**最终转给上游的头有哪些**
- 六项检查全绿,`test result:` 行贴进报告
