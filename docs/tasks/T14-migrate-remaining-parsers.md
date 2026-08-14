# T14:把 Gemini 与 OpenCode 也迁到共享流水线

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T14a 已合并**(接口扩完之前
> 这个任务做不了——2026-08-15 第一次派出去时执行者正确地停下来报告了,见 T14a §0)。
> 这是**纯重构**——**对外行为必须一字不变**。

---

## 目标

T1 建了共享流水线,T8 把 provider 的名字请出了 core。但 `ingest/` 下**只有两个
解析器**:

```
src-tauri/src/services/ingest/claude.rs      ✅ 已迁
src-tauri/src/services/ingest/codex.rs       ✅ 已迁
src-tauri/src/services/session_usage_gemini.rs    511 行  ❌ 还在各写各的
src-tauri/src/services/session_usage_opencode.rs  579 行  ❌ 还在各写各的
```

**「接一个新 agent = 写一个解析器」这个卖点目前只对两家兑现。** 另外两家还是
独立实现,意味着流水线里踩过的坑(增量游标、同秒 mtime、去重)它们没有继承,
以后改流水线要改三个地方。

这个任务就是把卖点兑现完。

---

## 1. 先读三样东西

1. `src-tauri/src/services/ingest/mod.rs` —— 流水线本体与 `SessionLogParser`
2. `src-tauri/src/services/ingest/codex.rs` —— **最复杂的那个实现**,覆盖了
   `prune` / `resolve_cursor` / `on_unchanged` 几个钩子,你要迁的两家大概率更简单
3. 你要迁的两个文件本身

**接口一个字都不要改。** 如果你发现某一家表达不了,**停下来报告** —— 那说明接口
有问题,不是你该临时加个钩子绕过去。

---

## 2. 目标结构

```
src-tauri/src/services/ingest/gemini.rs      新建
src-tauri/src/services/ingest/opencode.rs    新建
```

在 `ingest/mod.rs` 里 `pub mod gemini;` / `pub mod opencode;`。

原来那两个文件**保留为转发层**(照抄 T1 对 `session_usage.rs` 的处理):对外函数
签名不变,内部走新流水线。**不要删文件、不要改调用方** —— 别处还在用那些路径。

### 2.1 每家要实现什么

```rust
fn source(&self) -> &'static str;
fn log_roots(&self, home: &Path) -> Vec<PathBuf>;
fn is_log_file(&self, path: &Path) -> bool;
fn write_profile(&self) -> ProviderWriteProfile;   // T8 加的必填项
fn parse(&self, ctx: &LogFileContext<'_>) -> Result<Vec<ParsedUsage>, AppError>;
```

`prune` / `resolve_cursor` / `on_unchanged` / `record_file_error` /
`log_insert_failure` / `log_summary` 有默认实现,**只在这一家的语义确实不同时才覆盖**。

### 2.2 `write_profile` 的值照抄现有写库代码

两家现在写库时用的 `app_type`、`legacy_provider_id`、`provider_type`、
`agent_module_id`、`subscription_activity_id`、错误前缀 —— **逐字节照抄**,
不要重新起名。落库内容必须与重构前完全一致。

---

## 3. 保持不变的东西(违反即失败)

- ✅ 两个文件里现有的 **12 个测试**(各 6 个)**一字不改地继续通过**
- ✅ 对外函数签名不变,调用方一处不改
- ✅ 写进数据库的每一个字符串逐字节相同
- ✅ 两家各自的去重身份、游标语义**不要强行统一** —— 长得像但语义不同的
  就留在各自的解析器里(T1 §1.1 已经吃过这个亏)

**如果你发现必须改某个断言才能通过,停下来报告。**

---

## 4. 新增测试

1. 四个 parser 喂同一份流水线,各自的解析结果互不串味(扩展 T1 那条已有的两家版本)
2. gemini / opencode 各自的 `request_id` / `event_id` 与重构前逐字节相同
   (直接断言字面量)

---

## 5. 明确不要做的事

- ❌ 不要改 `SessionLogParser` 接口 —— 表达不了就停下来报告
- ❌ 不要动 `claude.rs` / `codex.rs` / `ingest/mod.rs` 的现有逻辑
  (只在 `mod.rs` 加两行 `pub mod`)
- ❌ 不要删除原来那两个文件
- ❌ 不要碰 `database/`(T15 在那里)
- ❌ 不要碰 `src/`(前端)、不要加新依赖

---

## 6. 完成的标准

- `ingest/` 下有四个解析器
- 原两个文件成为转发层,调用方零改动
- 12 个原测试一字未改地通过 + 2 条新测试
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- 报告里回答:**接第五个 agent 现在要写多少行、改哪些文件?**
