# T2:`subscription.rs` 按 provider 拆分

> **先读 [`README.md`](README.md) 的铁律。** 依赖:无。可与 T1、T3 并行。
> 这是**纯搬运**——**一行逻辑都不要改**。

---

## 目标

`src-tauri/src/services/subscription.rs` 现在一个文件里装着:共享类型、Codex 的凭据
读取与额度查询、Gemini 的凭据读取与额度查询。删掉某一家的支持时要在这个大文件里
动刀,而不是删文件。

拆成:

```
src-tauri/src/services/subscription/mod.rs      共享类型 + 分发入口
src-tauri/src/services/subscription/codex.rs    Codex 的全部
src-tauri/src/services/subscription/gemini.rs   Gemini 的全部
```

---

## 1. 怎么分

**留在 `mod.rs`(共享,不属于任何一家):**

- `CredentialStatus`、`QuotaTier`、`ExtraUsage`、`ManualResetCredit`、
  `ManualResetCredits`、`SubscriptionQuota`
- `SubscriptionQuota` 的 `skeleton` / `not_found` / `error` 三个构造器
- 所有 `TIER_*` 常量
- `window_seconds_to_tier_name` 等与具体厂商无关的辅助函数
- 分发入口 `get_subscription_quota(tool: &str)`
- `now_millis` 之类的小工具

**搬进 `codex.rs`:**

- `CodexAuthJson`、`CodexTokens`、`CodexCredentials` 及相关解析
- `read_codex_credentials*`、`parse_codex_credentials_json`
- `codex_chatgpt_base_url`、`normalize_chatgpt_base_url`、`codex_usage_url`、
  `codex_reset_credits_url`、`CHATGPT_OFFICIAL_BASE_URL`
- 429 冷却那一组(`codex_rate_limit_block` / `codex_is_rate_limited` /
  `record_codex_rate_limited`)
- `codex_wham_get`、`query_codex_quota`、`codex_usage_tiers`、`spend_control_utilization`、
  `normalize_codex_reset_credits` 及相关响应结构体
- `collect_codex_quota`
- 以及**它们对应的测试**

**搬进 `gemini.rs`:**

- Gemini 的凭据结构与读取、`refresh_gemini_token`、`query_gemini_quota`、
  `classify_gemini_model`、`collect_gemini_quota`
- 以及**它们对应的测试**

---

## 2. 拆完之后 `mod.rs` 的分发长这样(不要改成别的样子)

```rust
pub async fn get_subscription_quota(tool: &str) -> Result<SubscriptionQuota, String> {
    match tool {
        "claude" => crate::claude_quota::collect_local_quota(),
        "codex" => codex::collect_codex_quota().await,
        "gemini" => gemini::collect_gemini_quota().await,
        _ => Ok(SubscriptionQuota::not_found(tool)),
    }
}
```

**不要改成注册表/fn 指针表。** 三个分支各调一个具名函数,加一个工具是「写个模块 +
加一行」;注册表买到的只是省那一行,却换来装箱、间接层,还失去编译器的穷尽检查。

---

## 3. 保持不变的东西(违反即失败)

- ✅ **所有对外可见的路径必须继续可用**。别处是 `use crate::services::subscription::X`
  这样引用的,拆完之后这些引用**必须继续编译通过** —— 在 `mod.rs` 里 `pub use` 转出
  即可,**不要去改调用方**。
- ✅ **一行逻辑都不要改。** 这是搬运,不是重构。函数体原样搬过去。
- ✅ 现有测试全部继续通过,**断言一个字不改**。
- ✅ `pub(crate)` / `pub` 的可见性保持原样。

---

## 4. 明确不要做的事

- ❌ 不要顺手"优化"任何函数
- ❌ 不要改任何函数签名
- ❌ 不要动 `claude_quota.rs`(Claude 那家在别的文件里,这次不碰)
- ❌ 不要去修改调用方的 `use` 语句 —— 用 `pub use` 保持路径兼容
- ❌ 不要加新依赖
- ❌ 不要碰 `src/`(前端)

---

## 5. 怎么证明你没改逻辑

提交前跑一次,把输出贴进报告:

```bash
pnpm rust -- test
```

然后**逐个函数比对**:搬过去的函数体与原文件里的应当逐字相同(除了必要的 `use` 调整
和可见性修饰)。报告里说明:**有没有任何一处函数体发生了改变,如果有,是哪一处、为什么**。

---

## 6. 完成的标准

- 三个文件建好,原 `subscription.rs` 删除
- 调用方**一处未改**仍然编译通过
- 现有测试全部通过,断言未改
- 六项检查全绿,`test result:` 行贴进报告
- 报告里确认:函数体是否逐字搬运,有无例外
