# T1:会话日志抽出共享流水线

> **先读 [`README.md`](README.md) 的铁律。** 依赖:无。可与 T2、T3 并行。
> 这是**纯重构**——**对外行为必须一字不变**。

---

## 目标

现在两个 agent 的会话日志读取各写各的:

```
src-tauri/src/services/session_usage.rs        1,207 行  (Claude)
src-tauri/src/services/session_usage_codex.rs  2,075 行  (Codex)
```

但它们干的是同一套事,**只有「解析」是每家不同的**:

```
盯文件变化 → 读文件 → 解析成用量记录 → 去重 → 写库
   ↑共享     ↑共享      ↑ 每家不同     ↑共享  ↑共享
```

把共享的四步抽出来,每个 agent 只留一个解析器。**这样以后接一个新 agent,从
「照着抄 1,500 行」变成「写一个解析器」。**

---

## 1. 现状里已经开始的共享(先读懂再动)

`session_usage.rs` 里这几个函数**已经被 codex 那边复用了**:

- `get_sync_state`
- `load_sync_cursors`
- `metadata_modified_nanos`
- `update_sync_state`
- `update_sync_state_for_resource`

也就是说共享已经在发生,只是它们寄居在一个**以 Claude 命名的文件**里。这次要把它们
搬到中立的位置。

**动手前先通读这两个文件**,搞清楚哪些是真共享、哪些只是长得像。**长得像但语义不同的
不要强行合并** —— 宁可留两份,也不要造一个带一堆 `if is_codex` 的四不像。

---

## 1.1 三件必须先搞清楚的事实(接口就是照着它们设计的)

**这一节是本任务成败的关键。想当然会写出一个装不下 Codex 的接口。**

### ① 两家都是「整个文件重读」,不是「读增量」

代码里**没有任何 `seek`**。两家都是 `BufReader::new(file)` 从头读,用游标里的
`last_offset` 作**行号水位线**跳过已处理的行:

- Claude:`session_usage.rs:261`,`if line_offset <= last_offset { continue }`
  —— 在解析**之前**跳过
- Codex:`session_usage_codex.rs:798`,同样的判断,但位置在**更新累计基线之后**

### ② Codex 的位置为什么不一样:它的 token 是累计值

Codex 日志里的 `total_token_usage` 是**从会话开始累计**的,单次用量要靠
`compute_delta(&state.prev_total, &cumulative)` 算(`session_usage_codex.rs:773`)。
所以它**必须从文件第一行开始重建 `prev_total`**,哪怕这些行早就入过库 ——
跳过发生在算完基线之后。

**这一条直接否掉「给解析器一段增量文本」的设计**:只给增量,累计基线就没了,
第一条记录会被当成「从 0 涨到当前累计值」,用量直接翻几倍。

### ③ Codex 的去重身份要 `File` 句柄,不只要路径

`codex_file_identity(path, file, metadata)` 用的是 **device/inode**(Windows 上是
卷 + 文件索引)算出的哈希,目的是 rename 之后游标不串、路径复用时不沿用旧事件 ID。
所以解析器需要拿到**已打开的 `File` 和 `Metadata`**,光有路径不够。

两家的去重身份也**根本不是一回事**:

| | Claude | Codex |
| --- | --- | --- |
| `request_id` | 由 `message_id` 推出 | `codex_session:{scope}:{event_index}` |
| `event_id` | `claude-session:{message_id}` | `codex-session:{request_id}` |
| `upstream_correlation_id` | `Some(message_id)` | `None` |

**别指望一个「共用字段」的结构体能同时表达这两套。** 下面的接口让解析器
自己产出这些身份字符串,流水线只负责去重和写库。

---

## 2. 目标结构

新建:

```
src-tauri/src/services/ingest/mod.rs        流水线本体 + 共享的游标/去重/写库
src-tauri/src/services/ingest/claude.rs     Claude 的解析器
src-tauri/src/services/ingest/codex.rs      Codex 的解析器
```

`services/mod.rs` 里挂上 `pub mod ingest;`。

### 解析器接口(照写,不要改)

```rust
/// 流水线打开文件之后交给解析器的一切。
///
/// **`content` 是整个文件的内容，不是增量**——理由见 §1.1 ①②。
/// 增量靠 `last_line_offset` 这条水位线表达，由解析器自己决定在哪一步跳过
/// （Claude 在解析前跳，Codex 在重建累计基线之后跳）。
pub struct LogFileContext<'a> {
    pub path: &'a std::path::Path,
    /// 已打开的句柄与元数据——Codex 要靠它算 device/inode 身份（§1.1 ③）。
    pub file: &'a std::fs::File,
    pub metadata: &'a std::fs::Metadata,
    pub content: &'a str,
    /// 游标里记的行号水位线；`<= ` 它的行已经入过库。
    pub last_line_offset: i64,
}

/// 一条用量记录的去重身份。两家语义不同，**用枚举保持它们分开**，
/// 不要压成一个「通用」字符串字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageIdentity {
    /// Claude：assistant 消息的 message_id。
    ClaudeMessage { message_id: String },
    /// Codex：文件身份作用域 + 文件内事件序号。
    CodexEvent { scope: String, event_index: u32 },
}

/// 一条从会话日志里解析出来的用量记录，已经归一化。
///
/// 字段以现有代码实际写进库的那些为准——**不要新增或删减语义**。
/// 这是纯重构，落库内容必须与重构前完全一致。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUsage {
    pub identity: UsageIdentity,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// 事件发生时刻（秒）。解析不出来时由解析器按现有代码的兜底逻辑填。
    pub occurred_at: i64,
    /// 会话 ID。**注意两家用法不同**：写库时它进 legacy 字段，
    /// 不能当跨源去重键——照抄现有代码的注释和用法。
    pub session_id: Option<String>,
    /// 这一行在文件里的行号，供流水线推进水位线。
    pub line_offset: i64,
}

impl UsageIdentity {
    /// 写库用的 request_id / event_id / upstream_correlation_id。
    /// **三个都必须与重构前逐字节相同**，值照抄现有代码里的 format! 字面量。
    pub fn request_id(&self) -> String;
    pub fn event_id(&self) -> String;
    pub fn upstream_correlation_id(&self) -> Option<String>;
}

/// 每个 agent 只需要实现这一个东西。
pub trait SessionLogParser {
    /// 这个 agent 的标识，如 "claude" / "codex"。
    fn source(&self) -> &'static str;

    /// 要扫哪些目录。
    fn log_roots(&self, home: &std::path::Path) -> Vec<std::path::PathBuf>;

    /// 判断一个文件要不要读（扩展名、命名规则等）。
    fn is_log_file(&self, path: &std::path::Path) -> bool;

    /// 扫描前的剪枝。Codex 用它做日期分区剪枝，Claude 直接返回 `files` 原样。
    fn prune(&self, files: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> { files }

    /// 把一个文件解析成用量记录。返回的记录**已经排除了水位线以下的行**。
    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<Vec<ParsedUsage>, AppError>;
}
```

流水线本体提供一个入口,两家共用:

```rust
pub fn sync_with_parser(
    db: &Database,
    parser: &dyn SessionLogParser,
    bound_provider_id: Option<&str>,
) -> Result<SessionSyncResult, AppError>;
```

**注意返回类型**:`sync_claude_session_logs_bound` / `sync_codex_usage_bound` 对外返回的是
`ProviderSessionSyncResult`,不是 `SessionSyncResult`。流水线统一返回
`SessionSyncResult`,由那两个 bound 入口**照现有代码原样**转换成
`ProviderSessionSyncResult`(现在 `session_usage.rs:76` 附近就是这么做的,照抄)。

---

## 3. 保持不变的东西(违反即失败)

- ✅ `sync_claude_session_logs`、`sync_claude_session_logs_bound`、`sync_codex_usage`、
  `sync_codex_usage_bound`、`get_data_source_breakdown` **必须继续存在,签名不变**
  ——它们被别处调用。内部改成走新流水线即可。
- ✅ **现有 43 个测试必须一个不改地继续通过**(`session_usage.rs` 10 个,
  `session_usage_codex.rs` 33 个)。测试可以跟着代码搬家,但**断言内容一个字都不能改**。
  **如果你发现必须改某个断言才能通过,说明重构改变了行为,停下来报告。**
- ✅ 写进数据库的内容必须与重构前完全一致
- ✅ Codex 的日期分区剪枝、增量游标、同秒 mtime 的处理逻辑全部保留——那些是踩过坑
  补上的,不要"简化"

---

## 4. 明确不要做的事

- ❌ 不要改变任何对外行为,这是纯重构
- ❌ 不要"顺手优化"解析逻辑
- ❌ 不要为了统一而把两家语义不同的东西合并
- ❌ 不要动数据库 schema
- ❌ 不要加新依赖
- ❌ 不要碰 `src/`(前端)

---

## 5. 完成的标准

- 三个新文件建好并挂上
- 五个对外函数签名不变,内部走新流水线
- **43 个原有测试一字未改地通过**
- 新增至少 4 个测试:
  1. 同一份流水线喂两个不同 parser 时,各自的解析结果互不串味
  2. 两家的 `request_id` / `event_id` 与重构前逐字节相同(直接断言字面量,
     如 `"codex_session:{scope}:3"`、`"claude-session:{msg_id}"`)
  3. Codex 解析器在**水位线不为 0** 时,累计基线仍然从文件第一行重建
     (构造一个三行的累计序列,水位线设在第 2 行,断言第 3 行的增量是
     `total3 - total2` 而不是 `total3`)—— 这是本任务最容易写错的地方
  4. Codex 的文件身份来自 device/inode 而非路径(照抄现有那个同类测试的思路)
- 六项检查全绿,`test result:` 行贴进报告
- 报告里写明:哪些代码被判定为"真共享"搬进了 `mod.rs`,哪些"长得像但语义不同"
  被有意保留在各自的解析器里,以及为什么
