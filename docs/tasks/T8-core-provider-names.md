# T8:把 provider 的名字请出 core

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T1 已合并。可与 T6 并行
> (零文件重叠:T6 在 `src-tauri/src/router/`,你在 `src-tauri/src/services/ingest/`)。
> 这是**纯重构**——**落库内容必须一字节不变**。

---

## 目标

T1 把两家的会话日志读取抽成了共享流水线,做得不错。但 core(`ingest/mod.rs`)里
还留着一批 provider 的名字:

```
mod.rs:114   format!("codex_session:{scope}:{event_index}")
mod.rs:122   UsageIdentity::ClaudeMessage { .. } => format!("claude-session:{message_id}")
mod.rs:493   ProviderWriteProfile { app_type: "claude", legacy_provider_id: "_session", ... }
mod.rs:502   ProviderWriteProfile { app_type: "codex",  legacy_provider_id: "_codex_session", ... }
```

这违反 `docs/design/2026-08-14-modular-core-and-providers.md` §4:
**core 里不能出现任何 provider 的名字。**

### 为什么这不是洁癖

模块化的整个卖点是「**接一个新 agent = 写一个解析器**」。按现在的样子,接第三家
(gemini / opencode)**必须回来改 core 的两个 match**:加一个枚举变体、加一个
`ProviderWriteProfile` 分支。少改一处就是漏一家。

**债不还,T1 交付的那个卖点就是假的。**

---

## 1. 这笔债的形状(先看清楚,它比看起来小)

T1 已经把写库的 provider 参数收进了一个结构体:

```rust
struct ProviderWriteProfile<'a> {
    app_type: &'a str,
    legacy_provider_id: &'a str,
    provider_type: &'a str,
    insert_error_prefix: &'a str,
    calculator_app: Option<&'a str>,
    agent_module_id: &'a str,
    subscription_activity_id: &'a str,
}
```

**这个结构体的定义留在 core 是对的** —— 它是「core 需要知道哪些参数」的契约。
错的只有两件事:

1. **选哪一份**(`fn write_profile(identity: &UsageIdentity)` 的那个 match)
2. **身份字符串怎么拼**(`UsageIdentity` 的三个方法)

两件都应该由**解析器**说了算。

---

## 2. 改法(照做,不要自由发挥)

### 2.1 `UsageIdentity` 从枚举改成结构体

**现在**(core 认识两家):

```rust
pub enum UsageIdentity {
    ClaudeMessage { message_id: String },
    CodexEvent { scope: String, event_index: u32 },
}
impl UsageIdentity {
    pub fn request_id(&self) -> String { /* match 两家 */ }
    pub fn event_id(&self) -> String { /* match 两家 */ }
    pub fn upstream_correlation_id(&self) -> Option<String> { /* match 两家 */ }
}
```

**改成**(core 只拿到已经拼好的串):

```rust
/// 一条用量记录的身份。**字符串由解析器拼好交进来**，core 不知道
/// 也不需要知道它们长什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentity {
    /// legacy 表的 request_id。
    pub request_id: String,
    /// usage_events 的 event_id。
    pub event_id: String,
    /// 上游关联 ID；没有就是 None。
    pub upstream_correlation_id: Option<String>,
    /// 写进 legacy 行 `message_id` 列的值；没有就是 None。
    pub message_id: Option<String>,
    /// 仅用于日志的短标识（插入失败时打这个）。
    pub log_label: String,
}
```

**三个方法全部删掉**,调用点改成直接读字段。

`claude.rs` 里构造时填:

```rust
UsageIdentity {
    request_id: format!("{SESSION_REQUEST_ID_PREFIX}{message_id}"),
    event_id: format!("claude-session:{message_id}"),
    upstream_correlation_id: Some(message_id.clone()),
    message_id: Some(message_id.clone()),
    log_label: message_id.clone(),
}
```

`codex.rs` 里填:

```rust
let request_id = format!("codex_session:{scope}:{event_index}");
UsageIdentity {
    event_id: format!("codex-session:{request_id}"),
    upstream_correlation_id: None,
    message_id: None,
    log_label: request_id.clone(),
    request_id,
}
```

**这五个字段的值必须与重构前逐字节相同。** 上面两段就是照抄现在 core 里那三个
方法的 match 分支,只是换了地方。

### 2.2 `write_profile` 变成 trait 方法

`ProviderWriteProfile` 改成 `pub(crate)`,字段也改成 `pub(crate)`,让解析器能构造。
**生命周期参数去掉,改成 `&'static str`** —— 两家填的本来就都是常量。

`SessionLogParser` 加一个**必填**方法(不给默认实现,新接一家必须自己声明):

```rust
/// 这个 agent 写库时用的一组常量。**必填**：新接一个 agent 时，
/// 编译器会在这里逼你把它们说清楚，而不是让你漏掉 core 里的某个 match。
fn write_profile(&self) -> ProviderWriteProfile;
```

`fn write_profile(identity: &UsageIdentity)` 这个自由函数**删掉**,两个分支的内容
分别搬进 `claude.rs` / `codex.rs` 的 trait 实现里。

`CLAUDE_CODE_AGENT_MODULE_ID`、`CODEX_AGENT_MODULE_ID`、`CLAUDE_SUBSCRIPTION_ID`、
`CHATGPT_SUBSCRIPTION_ID` 的 `use` 一并从 `mod.rs` 挪到各自的解析器文件。

### 2.3 写库路径要拿得到 profile

`insert_usage_record` 等函数现在从 `identity` 推出 profile。改成**由调用方传进来**:

```rust
pub(crate) fn insert_usage_record(
    db: &Database,
    profile: &ProviderWriteProfile,
    parsed: &ParsedUsage,
    request_id: &str,
    bound_provider_id: Option<&str>,
) -> Result<bool, AppError>;
```

`sync_with_parser` 在循环外调一次 `parser.write_profile()`,往下传。

### 2.4 `log_insert_failure` 的默认实现

现在它 match 了 `UsageIdentity::CodexEvent` —— 一个号称「默认即 Claude 语义」的钩子
却认识 Codex。改成直接用 `record.identity.log_label`,match 删掉。

---

## 3. 验收:core 里搜不到 provider 名字

改完之后,这条命令**必须只剩两行 `pub mod` 声明**:

```bash
grep -in "claude\|codex\|gemini\|opencode" src-tauri/src/services/ingest/mod.rs
```

允许保留的例外**只有**:

- `pub mod claude;` / `pub mod codex;`(模块声明,不算)
- `#[cfg(test)] mod tests` 里的 `use` 与测试数据(测试当然要认识两家)

**除此之外一处都不许剩。** 报告里贴这条命令的实际输出。

---

## 4. 保持不变的东西(违反即失败)

- ✅ **写进数据库的每一个字符串都必须与重构前逐字节相同**:`request_id`、`event_id`、
  `upstream_correlation_id`、`message_id`、`app_type`、`legacy_provider_id`、
  `provider_type`、`calculator_app`、`agent_module_id`、`subscription_activity_id`
- ✅ 五个对外函数(`sync_claude_session_logs` 等)签名不变
- ✅ **47 个测试全部继续通过**

### 4.1 唯一允许改的测试,以及怎么改

`UsageIdentity` 的类型变了,所以**构造它**的测试代码必须跟着改。**只允许改构造,
不允许改断言**:

| 位置 | 怎么改 |
| --- | --- |
| `ingest/mod.rs` 的 `usage_identity_strings_match_pre_refactor_literals` | 改成构造新的结构体;**断言的那几个字面量一个字符都不许动** |
| `ingest/codex.rs:1946` 附近解构 `UsageIdentity::CodexEvent` 取 `event_index` 的那处 | 枚举没了,改成从 `identity.request_id` 里取,或在测试里自己记 index;**该测试原本断言什么,改完还断言什么** |

**其余 45 个测试一行都不许动。** 如果你发现还得改别的测试才能过,
**停下来报告** —— 那说明重构改变了行为。

---

## 5. 明确不要做的事

- ❌ 不要顺手把 gemini / opencode 也迁到这条流水线(不在本任务范围)
- ❌ 不要改解析逻辑、不要"优化"任何函数体
- ❌ 不要动数据库 schema
- ❌ 不要碰 `src-tauri/src/router/`(T6 正在那里工作)
- ❌ 不要碰 `src/`(前端)
- ❌ 不要加新依赖

---

## 6. 完成的标准

- `UsageIdentity` 是结构体,三个方法已删,字段由解析器填
- `write_profile` 是 `SessionLogParser` 的必填方法,自由函数已删
- §3 那条 grep 只剩两行 `pub mod` 声明(输出贴进报告)
- 47 个测试通过,其中只有 §4.1 允许的两处改了构造、断言未改
- 六项检查全绿,`test result:` 行贴进报告
- 报告里回答一个问题:**如果现在要接第三个 agent,需要改 core 里的哪些文件?**
  正确答案应该是「一个都不用改,只加一个解析器文件 + 在 `services/mod.rs` 挂一行」。
  如果不是,说明这次没改干净,写清楚差在哪。
