# T13:把 token 记进 `router_attempts`

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T6、T10、T11 已合并。
> 可与 T12 并行 —— 它只碰 `server.rs` 第 110 行之前和 `commands/router.rs`,
> 你碰第 110 行之后。**互相不要越界。**
>
> **这是整个 router 里最容易改坏的地方:流式透传路径。** §4「不许破坏的东西」
> 先读完再动手。

---

## 目标

T3 建 `router_attempts` 表时,`input_tokens` / `output_tokens` 两列的理由写在
设计文档 §5.1:

> 会话日志把 Codex 的用量整体绑到一个 provider,不区分实际走了哪家;
> **router 是全 app 唯一同时知道「多少 token」和「哪一家」的地方。**

但 T6 交付时这两列永远是 `None`(`server.rs:464`),因为它没解析响应流。
**表建了,列是空的,当初建表的主要理由没有兑现。**

你要做的是:在流式透传的同时把 usage 抓出来,回填到那一行。

---

## 1. token 在哪

Responses API 的 SSE 流里,最后有一个事件:

```
event: response.completed
data: {"type":"response.completed","response":{...,"usage":{"input_tokens":123,"output_tokens":456,...}}}
```

`usage` 在 `data` 那行的 JSON 里,路径是 `response.usage.input_tokens` /
`response.usage.output_tokens`。

**非流式响应**(客户端没要 stream)的 body 本身就是那个 JSON,`usage` 在顶层
`usage` 字段。两种都要处理。

---

## 2. 怎么做(照做,不要自由发挥)

### 2.1 `record_attempt` 要把行号交出来

现在它把 `record_router_attempt` 的返回值丢了(`server.rs:468`)。
改成返回 `Option<i64>`(失败时 `None` 并照旧记 log),成功路径把这个 id 留着。

### 2.2 DAO 加一个回填方法

`src-tauri/src/database/dao/router.rs`,**只加这一个方法,别的一行不要动**:

```rust
/// 把一次尝试的 token 数回填上去。行不存在时不报错——那一行可能因为
/// 写库失败根本没进去，为此让一个已经成功的请求失败不划算。
pub fn update_router_attempt_tokens(
    &self,
    id: i64,
    input_tokens: i64,
    output_tokens: i64,
) -> Result<(), AppError>;
```

### 2.3 边转发边扫,不要缓存整个响应

**这一条是硬要求。** T6 §4.1 定死了流式透传不整体缓存,你不许破坏它。

做法:在透传的 stream 上包一层,**每个 chunk 原样往下游发,同时喂给一个
增量 SSE 扫描器**。扫描器只保留「当前这个未完成的事件」,事件一完整就解析、
用完就丢。

```rust
/// 增量 SSE 扫描器：喂字节，吐出完整事件的 data 内容。
/// **内部缓冲有上限**——超过就丢弃当前事件继续找下一个，
/// 绝不能因为上游发了一个畸形的超大事件把内存吃光。
const MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
```

**绝对不要**把整个响应收集起来最后统一解析 —— 那正是 T6 §6 明令禁止的。

### 2.4 抓到之后

拿到 `usage` 就调 `update_router_attempt_tokens`。

- **回填失败只记 log,不影响请求** —— 请求已经成功了,记账失败不能变成用户可见的错误
- 一次请求**只回填一次**,拿到第一个 `response.completed` 就不再找
- 流断了、没等到 `response.completed`(用户中途取消、上游断流)→ **不回填**,
  两列保持 `None`。**不要猜、不要按 chunk 数估算。**

### 2.5 非流式响应

body 不大且已经在手里的情况下,直接解析顶层 `usage`。
**但同样受 §2.3 的上限约束** —— 不要为了解析而把一个超大 body 全读进内存,
超过 `MAX_SSE_EVENT_BYTES` 就放弃回填。

---

## 3. 测试

**不要起真实的上游服务。** T6 已经把「发请求」抽成可替换的函数,照它的测试写法。

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | SSE 流里有 `response.completed` 带 usage | 那一行的两列被回填成对应值 |
| 2 | usage 跨两个 chunk 被切开(在 JSON 中间断开) | 仍然能拼回来并正确回填 |
| 3 | 流里没有 `response.completed` | 两列保持 `None`,**不报错** |
| 4 | `response.completed` 里没有 `usage` 字段 | 两列保持 `None`,不 panic |
| 5 | 非流式响应,顶层有 `usage` | 正确回填 |
| 6 | 单个事件超过 `MAX_SSE_EVENT_BYTES` | 丢弃它继续找,不 OOM、不 panic |
| 7 | **透传的字节与上游发的逐字节相同** | 用一段含多事件的流,断言下游收到的字节序列与上游发出的完全一致 |

**第 2 条和第 7 条是这个任务的核心。** 第 2 条因为 SSE 的 chunk 边界和事件边界
毫无关系,切在 JSON 中间是常态;第 7 条因为你在改的是**用户实际看到的字节流**,
多一个字节少一个字节都是 bug。

---

## 4. 不许破坏的东西(违反即失败)

- ✅ **流式透传不整体缓存**(T6 §4.1)
- ✅ **透传字节逐字节不变** —— 你只是旁路观察,不是中间人改写
- ✅ **不引入额外延迟** —— chunk 必须先发给下游再喂扫描器,不要反过来
- ✅ `already_streaming` 的语义不变(决定 15)
- ✅ 无定时器(T6 §6)
- ✅ 生产代码无 `unwrap` / `expect`(T6 §7)

---

## 5. 明确不要做的事

- ❌ **不要碰 `server.rs` 第 110 行之前的代码** —— T12 正在那里工作
- ❌ 不要改 `router_attempts` 的表结构(两列已经在了)
- ❌ 不要把 token 也写进 `usage_events` —— 那张表管总量,这张管分账,
  **语义不同,不要相加**(T3 §1)
- ❌ 不要为了拿 usage 去改请求体(比如加 `stream_options`)——
  那会改变发给上游的内容
- ❌ 不要碰 `src/`(前端)
- ❌ 不要加新依赖

---

## 6. 完成的标准

- 成功的请求会把 token 回填进 `router_attempts`
- 流式路径仍然是流式的,透传字节逐字节不变
- 抓不到 usage 时两列保持 `None`,不猜不估
- 七条测试通过,其中第 2、7 条是核心
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- 报告里回答:**一次请求经过 router 之后,`sum_router_usage_by_provider`
  能不能给出正确的按 provider 分账?** 不能的话还差什么。
