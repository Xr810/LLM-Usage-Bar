# T23:SSE 流不再攒那 256 KB 兜底缓冲

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T13、T22 已合并。
> 可与 T24、T25 并行(你只碰 `route/server.rs` 的流式段)。
>
> **这是改流式透传路径 —— router 里最容易改坏的地方。** T13 的四条硬线一条都不能破。

---

## 0. 为什么要改(T22 实测的数字)

T13 为了给**非流式响应**兜底解析 usage,在流式路径上攒了一份 body 副本,
上限 `MAX_SSE_EVENT_BYTES = 256 KB`。T22 量了它的代价:

| 量 | 数字 |
| --- | --- |
| 攒满 256 KB 的拷贝成本 | **13.3–18.0 µs / 请求** |
| 超限 `clear()` 之后**仍然占着的容量** | **262,144 B** —— `Vec::clear()` 不释放容量,一直占到流结束 |
| 兜底路径:整份 256 KB 顶层 JSON 解析 | **1.08 ms**(min 0.87 / max 2.19) |
| 内存上界 | 每个在飞请求 `min(流字节数, 256 KiB)`,并发 10 → ≤ 2.5 MiB |

**第二行是关键**:超限之后那 256 KB 并没有还回去,它占到流结束为止。

**而这份缓冲对 SSE 流是纯浪费** —— SSE 的 usage 走的是 `SseScanner` 的
`response.completed` 回填,顶层 JSON 兜底对 SSE 文本本来就解析不出任何东西。

---

## 1. 改法

响应的 content-type **已经拿在手里了**:

```rust
// route/server.rs:842
let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
```

在流式段里用它开一个门:**content-type 是 `text/event-stream` 时,完全不攒 `buffered`。**

判断要**宽松**:大小写不敏感,允许后面带参数(`text/event-stream; charset=utf-8`),
用 `starts_with` 或 `contains` 判 `text/event-stream` 即可。**拿不到 content-type
时按「攒」处理**(保守),不要按不攒。

---

## 2. 不许破的东西(T13 的四条硬线,原样继承)

- ✅ **流式透传不整体缓存**,chunk 先发下游再喂扫描器,顺序不能反
- ✅ **透传字节逐字节不变** —— 你是旁路观察者
- ✅ 抓不到 usage 就让两列保持 `None`,不猜不估
- ✅ 生产代码无 `unwrap` / `expect`,无定时器

**T13 的 7 条测试一条都不许改断言。** 其中第 5 条(非流式顶层 usage)用的是
`application/json`,它必须继续走兜底路径并回填成功 —— **这条是这次改动的安全网**。

---

## 3. 新增测试

1. content-type 是 `text/event-stream` 时,usage 仍从 `response.completed` 正确回填
   (证明 SSE 路径不受影响)
2. content-type 是 `text/event-stream; charset=utf-8` 时同样(**带参数**)
3. content-type 是 `application/json` 时,顶层兜底仍然工作(T13 第 5 条的加强版)
4. **没有 content-type 头**时,按保守路径处理(仍然攒、兜底仍可用)

---

## 4. 已知的语义边角(接受,写进注释)

上游把一个 **JSON 错误体误标成 `text/event-stream`** 时,兜底回填会丢掉 ——
两列保持 `None`,**不会崩、不会算错**。这是 T22 提案时就点明的取舍,
**接受**,但要在代码注释里写清楚,别让后面的人以为是 bug。

---

## 5. 明确不要做的事

- ❌ 不要改 `SseScanner` 的任何逻辑
- ❌ 不要动 `MAX_SSE_EVENT_BYTES` 这个常量的值(非流式路径还要用)
- ❌ 不要顺手把 `buffered` 换成别的容器 —— 只加门,不换实现
- ❌ 不要碰 `route/server.rs` 之外的文件
- ❌ 不要加新依赖

---

## 6. 完成的标准

- SSE 响应下 `buffered` 全程为空(**用测试证明**,不是靠读代码)
- T13 的 7 条测试断言未改且全过
- 4 条新测试通过
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- 报告里给出:改动后 SSE 路径的拷贝成本与驻留内存(对照 §0 那张表)
