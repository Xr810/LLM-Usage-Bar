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
盯文件变化 → 读增量 → 解析成用量记录 → 去重 → 写库
   ↑共享      ↑共享      ↑ 每家不同      ↑共享  ↑共享
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
/// 一条从会话日志里解析出来的用量记录，已经归一化。
/// 字段以现有代码实际写进库的那些为准——**不要新增或删减字段**，
/// 这是纯重构，落库内容必须与重构前完全一致。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUsage {
    // 照抄现有两个文件写库时用到的字段
}

/// 每个 agent 只需要实现这一个东西。
pub trait SessionLogParser {
    /// 这个 agent 的标识，如 "claude" / "codex"。
    fn source(&self) -> &'static str;

    /// 要扫哪些目录。
    fn log_roots(&self, home: &std::path::Path) -> Vec<std::path::PathBuf>;

    /// 判断一个文件要不要读（扩展名、命名规则等）。
    fn is_log_file(&self, path: &std::path::Path) -> bool;

    /// 把一段新增内容解析成用量记录。
    /// `content` 是自上次游标以来的增量文本。
    fn parse(&self, path: &std::path::Path, content: &str)
        -> Result<Vec<ParsedUsage>, AppError>;
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
- 新增至少 3 个测试:同一份流水线喂两个不同 parser 时,各自的解析结果互不串味
- 六项检查全绿,`test result:` 行贴进报告
- 报告里写明:哪些代码被判定为"真共享"搬进了 `mod.rs`,哪些"长得像但语义不同"
  被有意保留在各自的解析器里,以及为什么
