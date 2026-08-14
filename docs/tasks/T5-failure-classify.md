# T5:失败分类(纯函数,无 IO)

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T3 必须已完成并合并**。
> 可与 T4 并行。

---

## 目标

写一个**纯函数**:给定一次上游请求的结果,判断

1. 这算不算失败
2. 失败了要不要**换下一家**
3. 要不要**拉黑**,拉黑谁(这一行 还是 整个 provider),拉多久

**不碰数据库、不发网络、不读时间。** 输入全是参数,输出是一个决定。

---

## 1. 新建文件

`src-tauri/src/router/failure.rs`,并在 `src-tauri/src/router/mod.rs` 里
`pub mod failure;`。

---

## 2. 类型与签名(照写,不要改)

```rust
/// 一次上游尝试的结果，由调用方(T6)从真实响应里提取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptResult {
    /// 拿到了 HTTP 响应。
    Http { status: u16, body_snippet: String },
    /// 连不上、DNS 失败、TLS 失败。
    ConnectFailed,
    /// 连上了但迟迟没有第一个字节。
    FirstByteTimeout,
    /// 流已经开始吐字之后断了。
    StreamBroken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Connect,
    Timeout,
    RateLimited,
    ServerError,
    ModelNotFound,
    /// 认证错、参数错、内容被拒等——换一家也是同样的错。
    RequestRejected,
    StreamBroken,
}

/// 拉黑的作用范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlacklistScope {
    /// 只拉黑 (这个 provider, 这个模型) 这一行。
    ThisRoute,
    /// 拉黑整个 provider——只用于与模型无关的故障。
    WholeProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureVerdict {
    pub kind: FailureKind,
    /// 是否应当继续尝试队列里的下一家。
    pub try_next: bool,
    /// 是否拉黑；None 表示不拉黑。
    pub blacklist: Option<BlacklistScope>,
    /// 拉黑时长（秒）。仅当 blacklist 为 Some 时有意义。
    pub cooldown_secs: i64,
}

/// 成功返回 None，失败返回判定。
///
/// `already_streaming`：本次请求是否已经向客户端吐过字。
pub fn classify(result: &AttemptResult, already_streaming: bool) -> Option<FailureVerdict>;
```

---

## 3. 判定表(逐行实现,逐行测试)

**先看这一条,它压倒其他所有规则:**

> **`already_streaming == true` 时,`try_next` 永远是 `false`。**
> 已经有半句话到用户屏幕上了,换一家会让他看到两截拼在一起的话。
> 这是流式协议的固有限制(设计文档决定 15)。

| 输入 | kind | try_next | blacklist | cooldown |
| --- | --- | --- | --- | --- |
**按这个顺序判,先命中先返回**(429 和 404 都落在 4xx 区间里,顺序错了就归错类):

| # | 输入 | kind | try_next | blacklist | cooldown |
| --- | --- | --- | --- | --- | --- |
| 1 | `Http { status: 200..=299 }` | — | — | 返回 `None`(成功) | — |
| 2 | `ConnectFailed` | `Connect` | ✅ | `WholeProvider` | 60 |
| 3 | `FirstByteTimeout` | `Timeout` | ✅ | `WholeProvider` | 60 |
| 4 | `StreamBroken` | `StreamBroken` | ❌ | `None` | — |
| 5 | `Http { 429 }` | `RateLimited` | ✅ | `ThisRoute` | 60 |
| 6 | `Http { 404 }` **或** `Http { 400 }` 且 body 含模型不存在的迹象 | `ModelNotFound` | ✅ | `ThisRoute` | **600** |
| 7 | 其余 `Http { 400..=499 }` | `RequestRejected` | ❌ | `None` | — |
| 8 | `Http { 500..=599 }` | `ServerError` | ✅ | `ThisRoute` | 60 |
| 9 | **其余一切 `Http`(1xx、3xx、≥600)** | `RequestRejected` | ❌ | `None` | — |

### 3.0 第 9 行为什么要有

上游正常工作时不该出现 1xx / 3xx —— reqwest 自己跟重定向,3xx 漏到这里说明
**配置错了(base_url 指到了一个会重定向的地方)**,换一家不会更好,拉黑还会掩盖问题。
所以归到「换一家也是同样的错」这一类,原样透传给客户端让用户看见。

**这一行不是可选的**:没有它,`classify` 对 302 就没有定义好的返回值,
实现者只能自己瞎猜或者 panic。

### 3.1 「模型不存在」怎么判

`body_snippet` 里(**忽略大小写**)包含下列任一子串时,算 `ModelNotFound`:

```
model_not_found
does not exist
unknown model
no such model
不存在
```

**只在 status 是 400 或 404 时才做这个判断。** 其他状态码不看 body。

### 3.2 为什么 `RequestRejected` 不换下一家

401/403(认证错)、400(参数错)、422(内容被拒)——**换一家只是把同一个错再挨
一遍**,还多等一轮。设计文档决定 14。

### 3.3 为什么模型不存在要拉黑 10 分钟而不是 1 分钟

「这家没有这个模型」是**稳定事实**,不是临时故障。1 分钟就重试等于反复撞同一堵墙。
但也不永久拉黑——中转随时可能上新模型。

---

## 4. 测试(必须写,而且必须过)

`#[cfg(test)] mod tests`,**至少覆盖**:

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | 200 | `None` |
| 2 | 204 | `None`(边界:2xx 全算成功) |
| 3 | `ConnectFailed` | `Connect` / try_next / `WholeProvider` / 60 |
| 4 | `FirstByteTimeout` | `Timeout` / try_next / `WholeProvider` / 60 |
| 5 | 429 | `RateLimited` / try_next / `ThisRoute` |
| 6 | 500、503 | `ServerError` / try_next / `ThisRoute` |
| 7 | 404 | `ModelNotFound` / try_next / `ThisRoute` / **600** |
| 8 | 400 + body 含 `"model_not_found"` | `ModelNotFound` |
| 9 | 400 + body 含 `"The model does NOT Exist"`(大小写混杂) | `ModelNotFound`(**大小写不敏感**) |
| 10 | 400 + body 是别的错 | `RequestRejected` / **不换** / 不拉黑 |
| 11 | 401 | `RequestRejected` / 不换 |
| 12 | 403 | `RequestRejected` / 不换 |
| 13 | 500 但 `already_streaming = true` | **try_next 为 false**,但仍然拉黑 |
| 14 | 429 但 `already_streaming = true` | **try_next 为 false** |
| 15 | `StreamBroken` | 不换、不拉黑 |
| 16 | 404 但 body 里没有任何关键词 | 仍然是 `ModelNotFound`(404 本身就够) |
| 17 | 302 | `RequestRejected` / 不换 / 不拉黑(判定表第 9 行) |
| 18 | 100 | `RequestRejected` / 不换 / 不拉黑 |
| 19 | `ConnectFailed` 且 `already_streaming = true` | **try_next 为 false**,但仍然 `WholeProvider` 拉黑 60 秒 |

第 13、14、19 条是这个任务最容易写错的地方:**已经吐字了就不能换,但该拉黑还是要拉黑**
——下一次请求应该跳过这家。注意第 19 条:`already_streaming` 的压制是**全局**的,
不只作用于 HTTP 分支。

---

## 5. 明确不要做的事

- ❌ 不要读数据库、不要写拉黑记录(本任务只**产出判定**,写入是 T6 的事)
- ❌ 不要读系统时间(cooldown 是秒数,不是绝对时刻)
- ❌ 不要解析 `Retry-After`(那是 T6 在有响应头时做的事,本函数只看 status 和 body)
- ❌ 不要加新依赖

---

## 6. 完成的标准

- 文件建好并挂上 `pub mod failure;`
- 签名与本文完全一致
- 19 条测试全部通过
- 六项检查全绿,`test result:` 行贴进报告
