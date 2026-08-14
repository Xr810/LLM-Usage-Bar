# T14a:把解析器接口扩到能表达四家

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T8、T13 已合并。
> 可与 T15 并行(你在 `services/ingest/`,它在 `database/`)。
> **T14b(真正迁移 gemini/opencode)依赖本任务。**
>
> **这不是纯重构** —— 它改 core 的接口。但**现有两家的行为必须一字不变**,
> 47 个测试盯着。

---

## 0. 这个任务是怎么来的

T14 通读之后**停下来报告**:现在的 `SessionLogParser` 表达不了 gemini/opencode,
硬迁就得改断言或者发明名字。**它是对的**,四条已复核:

| # | 它需要什么 | 现状 |
| --- | --- | --- |
| O1 | opencode 的费用是上游给的(`session_usage_opencode.rs:352-362`,`msg.cost > 0` 时直接写进 `total_cost_usd`) | `ParsedUsage` 没有费用字段,费用只能来自定价表 |
| O2 | opencode 的变更判定要看 `-wal` 的 mtime(`:64-73`,注释明写「否则会在 checkpoint 之前漏掉刚写入的会话」) | 流水线只看主库 mtime |
| O3 | opencode 一个文件里多个会话,各自有水位、各自推进 | 流水线每文件只写一条游标 |
| O4 | opencode 单条插入失败要进 `result.errors` 并继续 | 只 log,不进 errors |
| O5 | gemini/opencode 没有订阅,不该 `mark_subscription_activity` | `subscription_activity_id` 是必填 `&'static str`,每次插入无条件调 |

**O5 是我(任务书作者)在 T8 里定错的** —— 当时只有两家、两家都有订阅,就写成了必填。

---

## 1. 好消息:两个槽位仓库里已经有了

动手前先确认这两处,它们能省掉一半工作:

- **`usage_sync_cursors` 表已有 `parser_state_json` 列**(`dao/usage_sync_cursors.rs:17`)。
  流水线现在只是把旧值原样搬运(`ingest/mod.rs:777`),**没给解析器用**。
  O3 的会话级水位就存这里,不需要新表、不需要迁移。
- **写库路径已有 `upstream_cost` 概念**(`ingest/mod.rs:518`,现在恒为 `None`)。
  O1 接这个槽位即可。

---

## 2. 五处改动(照写,不要自由发挥)

### 2.1 `ParsedUsage` 加上游费用

```rust
pub struct ParsedUsage {
    // …现有字段一个不动…
    /// 上游已经算好的总费用。`Some` 时直接落库、不再查定价表。
    /// 类型与写库列一致——照抄现在 total_cost_usd 那一列用的类型。
    pub upstream_total_cost: Option<String>,
}
```

**legacy 路径**(`ingest/mod.rs:574-590`):`Some(cost)` 时**跳过定价表**,
写成 `("0", "0", "0", "0", cost)` —— 这是 `session_usage_opencode.rs:356-362`
现在的形状,**逐字节照抄它,包括那四个 `"0"`**。

**bound 路径**:把值传给已有的 `upstream_cost` 字段。

`claude.rs` / `codex.rs` 一律填 `None`,行为不变。

### 2.2 变更判定可以纳入辅助文件

```rust
/// 除了日志文件本身，还有哪些文件的 mtime 也算「这个文件变了」。
/// 默认空。SQLite 类日志用它把 `-wal` 纳进来。
fn extra_change_sources(&self, _path: &Path) -> Vec<PathBuf> { Vec::new() }
```

流水线在算 `file_modified` 时取**主文件与全部 extra 的最大值**。
默认空 ⇒ claude/codex 的判定值一模一样。

### 2.3 解析器可以持有跨轮状态

`LogFileContext` 加一个字段:

```rust
/// 上一轮这个解析器存下的私有状态；首次为 None。
/// 流水线**只存不看**,内容格式由解析器自己定。
pub parser_state: Option<&'a str>,
```

`parse` 的返回类型改成:

```rust
pub struct ParseOutput {
    pub records: Vec<ParsedUsage>,
    /// 要求下次带回来的状态。`None` 表示保持上一轮的值不变。
    pub next_state: Option<String>,
}

fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError>;
```

**这会改 `claude.rs` / `codex.rs` 的 `parse` 签名** —— 那是机械改动:
把原来的 `Ok(records)` 改成 `Ok(ParseOutput { records, next_state: None })`,
**函数体其余部分一行不动**。

流水线把 `next_state` 写进游标的 `parser_state_json` 列(`Some` 才写,
`None` 保持原值)。

### 2.4 插入失败可以进 `errors`

`log_insert_failure` 的返回值从 `()` 改成 `Option<String>`:

```rust
/// 单条记录插入失败时的处理。返回 `Some(msg)` 表示这条也要进
/// `SessionSyncResult::errors`；返回 `None` 表示只记日志。
/// 默认实现返回 `None`——即 claude/codex 现在的行为。
fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String>;
```

### 2.5 订阅活动改成可选

```rust
pub(crate) struct ProviderWriteProfile {
    // …
    /// `None` 表示这家没有订阅概念，插入后**不调**
    /// `mark_subscription_activity`。
    pub(crate) subscription_activity_id: Option<&'static str>,
}
```

`ingest/mod.rs:644` 那句改成只在 `Some` 时调。
`claude.rs` / `codex.rs` 填 `Some(原来那个常量)`,行为不变。

---

## 3. 保持不变的东西(违反即失败)

- ✅ **47 个现有测试一字不改地通过**
- ✅ claude / codex 写进数据库的每一个字符串逐字节相同
- ✅ 五个对外函数签名不变,调用方一处不改
- ✅ 默认实现必须等价于「claude/codex 现在的行为」——
  每加一个带默认值的钩子,都要问一遍「两家用默认值时行为变了吗」

**如果你发现必须改某个断言才能通过,停下来报告。**

---

## 4. 新增测试

1. `upstream_total_cost = Some("1.25")` 时,legacy 路径落库的五个费用列是
   `("0","0","0","0","1.25")`,**不查定价表**
2. `upstream_total_cost = None` 时,费用与本任务之前完全相同(拿一个已有
   claude 用例断言前后一致)
3. `extra_change_sources` 返回一个更新更晚的文件时,`file_modified` 取到的是
   那个更晚的值
4. `next_state = Some(x)` 会被写进游标的 `parser_state_json`;下一轮
   `ctx.parser_state` 读到 `Some(x)`
5. `next_state = None` 时游标里原有的 `parser_state_json` 不被清掉
6. `log_insert_failure` 返回 `Some` 时那条消息出现在 `result.errors` 里
7. `subscription_activity_id = None` 时不调 `mark_subscription_activity`
   (用一个可观察的替身或计数器验证)

---

## 5. 明确不要做的事

- ❌ **不要在本任务里迁 gemini/opencode** —— 那是 T14b。本任务只扩接口,
  并让现有两家继续绿
- ❌ 不要改 `usage_sync_cursors` 的表结构(`parser_state_json` 列已经在了)
- ❌ 不要动 `database/`(T15 在那里)、不要动 `router/`
- ❌ 不要重排目录(那是 T17)
- ❌ 不要碰 `src/`、不要加新依赖

---

## 6. 完成的标准

- 五处改动全部落地,默认实现等价于现有行为
- 47 个原测试一字未改地通过 + 7 条新测试
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- 报告里逐条回答 T14 提出的 O1–O5:**现在这五条各自能表达了吗?**
  还有哪一条表达不了,说清楚差在哪 —— T14b 会按你的答案开工
