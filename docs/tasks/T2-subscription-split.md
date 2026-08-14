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

下面是**逐项穷举**,按 `subscription.rs` 里的出现顺序排。**这张表就是全部** ——
文件里没有第四类东西。搬完对一遍:每一项都必须落在三个文件之一。

**留在 `mod.rs`(共享,不属于任何一家):**

| 行 | 项 |
| --- | --- |
| 21 | `CredentialStatus` |
| 33 | `QuotaTier` |
| 51 | `ExtraUsage` |
| 62 | `ManualResetCredit` |
| 75 | `ManualResetCredits` |
| 83 | `SubscriptionQuota` + 它的 `skeleton` / `not_found` / `error` 构造器 |
| 156–179 | **所有** `TIER_*` 常量,**包括 `TIER_GEMINI_*`** |
| 432 | `window_seconds_to_tier_name` |
| 452 | `unix_ts_to_iso` |
| 1458 | 分发入口 `get_subscription_quota` |
| 1471 | `now_millis` |

`TIER_GEMINI_*` 留在 `mod.rs` 是**有意的**,不是漏了:它们是 `pub` 的对外契约,
和别家的 `TIER_*` 属于同一组命名空间,分散到子模块只会让调用方要记「哪个常量在哪」。
**常量按契约归类,函数按厂商归类。**

**搬进 `codex.rs`:**

| 行 | 项 |
| --- | --- |
| 184 | `CodexAuthJson` |
| 191 | `CodexTokens` |
| 197 | `CodexCredentials`(**是 type alias,不是 struct**) |
| 211 / 224 / 244 | `read_codex_credentials` / `_from_keychain` / `_from_file` |
| 267 | `parse_codex_credentials_json` |
| 335 | `is_codex_token_stale` |
| 352–425 | `CodexRateLimitWindow`、`CodexRateLimit`、`CodexAdditionalRateLimit`、`CodexSpendControl`、`CodexSpendControlLimit`、`CodexUsageResponse`、`CodexResetCreditSummary`、`CodexResetCredit`、`CodexResetCreditTimestamp`、`CodexResetCreditsResponse` |
| 456 | `codex_reset_credit_timestamp_to_iso` |
| 467 | `codex_additional_rate_limit_tier_name` |
| 471 | `codex_usage_tiers` |
| 560 | `spend_control_utilization` |
| 573 | `normalize_codex_reset_credits` |
| 621 / 638 / 640 | `codex_chatgpt_base_url`、`CHATGPT_OFFICIAL_BASE_URL`、`normalize_chatgpt_base_url` |
| 665 / 673 | `codex_usage_url`、`codex_reset_credits_url` |
| 678–698 | 429 冷却那一组:`CODEX_RATE_LIMIT_DEFAULT_BLOCK_SECS`、`codex_rate_limit_block`、`codex_is_rate_limited`、`record_codex_rate_limited` |
| 701 | `codex_wham_get` |
| 725 | **`query_managed_codex_oauth_quota`** —— `pub(crate)`,被 `src/usage/quota.rs` 直接引用,**必须 `pub use` 转出去** |
| 770 | `query_codex_quota` —— 同样 `pub(crate)`,同样被 `usage/quota.rs` 引用,**必须转出** |
| 1365 | `collect_codex_quota` |

**搬进 `gemini.rs`:**

| 行 | 项 |
| --- | --- |
| 905 | `GeminiOAuthCredsFile` |
| 926 / 939 / 1028 | `read_gemini_credentials` / `_from_keychain` / `_from_file` |
| 972 / 1055 | `parse_gemini_keychain_json`、`parse_gemini_file_json` |
| 1103 / 1105 | `GEMINI_OAUTH_CLIENT_ID`、`GEMINI_OAUTH_CLIENT_SECRET` |
| 1111 | `refresh_gemini_token` |
| 1139 / 1146 / 1157 | `GeminiLoadCodeAssistResponse`、`GeminiBucketInfo`、`GeminiQuotaResponse` |
| 1162 | `extract_project_id` |
| 1175 | `classify_gemini_model` |
| 1192 | `query_gemini_quota` |
| 1405 | `collect_gemini_quota` |

**测试**:文件末尾的 9 个测试跟着**它们测的那个函数**走。测的是共享类型的留
`mod.rs`。搬过去之后 `use super::*` 可能不够,补 `use` 即可,**断言一个字不改**。

### 1.1 `pub use` 转出清单

`mod.rs` 里必须有,否则 `src/usage/quota.rs:6` 的 `use` 会断:

```rust
pub(crate) use codex::query_managed_codex_oauth_quota;
```

**只转这一个。** `query_codex_quota` 虽然也是 `pub(crate)`,但模块外没有任何
调用方(`codex_oauth.rs` 里只有一句提到它的注释),转出去反而会触发
`unused_imports`,在 `-D warnings` 下直接挂。

**可见性照原样保留**:`query_codex_quota` 在 `codex.rs` 里仍然是 `pub(crate) fn`,
只是不从 `mod.rs` 转出 —— 没人用那条路径,所以不算破坏兼容。

搬完先只跑 `pnpm rust -- check`,编译过了再往下做。**编译错误就是你的转出清单还缺项**,
不要去改调用方。

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
