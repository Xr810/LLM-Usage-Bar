# provider 模块的目录形态,以及「功能域」这一层要不要建

> **状态:2026-08-15 定稿。** 回答
> [`2026-08-14-modular-core-and-providers.md`](2026-08-14-modular-core-and-providers.md)
> §9 留下的问题,以及 [`../../HANDOFF.md`](../../HANDOFF.md) §15.2 明确点名的
> 「先决定 provider 模块的目录形态,再把剩下的文件归位」。
>
> **本文只定形态,不排期、不派活。** 落地任务见
> [`../tasks/T21-provider-modules-and-sync.md`](../tasks/T21-provider-modules-and-sync.md)。

---

## 1. 触发本文的问题

T17 之后 `services/` 还剩 17 个文件、11,263 行。HANDOFF §15.2 说它们
「不属于 §9 的八个模块里的任何一个」,并要求**先决定 provider 模块长什么样**。

同时有一个更大的问题一直悬着(模块化设计 §9.2 那张扁平图之后没人回答):

> **第一层到底按技术层切(`model/store/quota/route/...`),还是按功能域切?**

2026-08-15 的讨论里我的判断是「等第二个功能域真正落地再定,现在没有用例」。
**盘完 `services/` 之后这个判断需要修正:第二个功能域早就在仓库里了。**

---

## 2. 关键发现:`services/` 剩下的是两堆,不是一堆

逐个文件归类(行数为 2026-08-15 实测):

### 2.1 第一堆:provider 模块(7 项,约 5,700 行)

| 文件 | 行 | 是什么 |
| --- | --- | --- |
| `coding_plan.rs` | 2,192 | Kimi / GLM / MiniMax 的 Token Plan 额度 |
| `system_provider_connection.rs` | 1,085 | provider 连通性与 key usage 抓取 |
| `subscription/codex.rs` | 898 | Codex 订阅额度 |
| `provider_key_usage_scheduler.rs` | 599 | key usage 的刷新调度 |
| `subscription/gemini.rs` | 512 | Gemini 订阅额度 |
| `official_pricing.rs` | 465 | 官方价目表刷新 |
| `balance.rs` | 454 | DeepSeek / StepFun / SiliconFlow / **OpenRouter** / Novita 余额 |
| `claude_cli_auth.rs` | 453 | Claude CLI 认证 |
| `subscription/mod.rs` | 331 | 共享类型 + `get_subscription_quota` |

**这一堆是「同一件事的不同厂商实现」—— 横向的。**

### 2.2 第二堆:备份同步(8 项,约 3,800 行)

| 文件 | 行 |
| --- | --- |
| `s3.rs` | 926 |
| `sync_protocol.rs` | 710 |
| `webdav.rs` | 554 |
| `webdav_sync/archive.rs` | 424 |
| `webdav_sync.rs` | 335 |
| `s3_sync.rs` | 319 |
| `webdav_auto_sync.rs` | 267 |
| `s3_auto_sync.rs` | 263 |

**这一堆是一整块独立业务 —— 纵向的。** 它跟用量监控没有任何关系。

实测它的全部上行依赖:

```
crate::store (12) · crate::error (7) · crate::config (6)
crate::http_client (4) · crate::product_identity (1) · crate::services (自己人)
```

**零 `usage` / `quota` / `route` / `ingest` / `model`。**

### 2.3 第三类:一个孤儿

`budget_alert.rs`(456 行)—— 每日预算提醒,读托盘快照算好的百分比。
它属于**用量监控域**,不属于上面两堆。

---

## 3. 结论一:第二个功能域不是假设,它已经存在

模块化设计 §9.2 的目标划分把后端切成
`model / store / ingest / quota / route / api / config / secrets`。
但 `ingest`、`quota`、`route` 三者其实都是**「用量监控」这一个功能域内部**的层 ——
而备份同步既不用 ingest、也不用 quota、更不碰 route。

**所以那张扁平图混了两个维度**:前四个是功能域内部的层,后四个(`model`/`store`/
`config`/`secrets`)是真正跨域的地基。备份同步的依赖清单(§2.2)恰好证明了这条线在哪:
**它只依赖后四个。**

这不是需要等 session manager 来验证的假设。**它已经是仓库里跑着的代码。**

---

## 4. 决定

### 决定 1:建 `providers/` 和 `sync/` 两个平级目录,**不建 `features/` 这一层**

```
src/
  model/  store/  config/  secrets/  http_client.rs     ← 地基:跨域共享
  providers/                                            ← 新建:横向,各家实现
  usage/  ingest/  quota/  route/                       ← 功能域:用量监控
  sync/                                                 ← 新建:功能域:备份同步
  api/                                                  ← 契约 + 传输薄壳
```

**为什么不套 `features/` 前缀。** 它唯一的收益是「从路径就能看出谁是地基、谁是可拆的
功能域」。但那个信息**放进守卫里更好** —— 守卫是机器检查的,目录名是装饰,
而且改目录名要动全树的 `use`。

现在只有两个功能域,多一层目录换不来任何东西。**第三个功能域落地时重新评估**
(那时 `git mv` 两个目录进 `features/` 仍然是十分钟的事,这个决定不锁死未来)。

### 决定 2:`providers/` 与 `sync/` 是两种不同的东西,不许混为一谈

| | `providers/` | `sync/`(及将来的功能域) |
| --- | --- | --- |
| 方向 | 横向:同一件事的不同厂商 | 纵向:一整块独立业务 |
| 谁调谁 | **core 调它**,它不调业务层 | 它调地基,**没人调它** |
| 需要注册表吗 | **要**,core 得遍历 | **不要**,启动时接一次线 |
| 删掉一个的代价 | 删目录 + 删注册表一行 | 删目录 + 删启动接线几行 |

设计文档 §5 已经把 provider 模块与前端面板辨析过了;**本条是那张表缺的第三列。**

### 决定 3:`providers/` 内部按厂商分目录,注册表在 `providers/mod.rs`

```
providers/
  mod.rs            注册表:字符串键 → 实现。core 只认这个文件
  claude/           claude_cli_auth · ClaudeChainCollector · claude_quota
  codex/            subscription/codex · CodexChainCollector · codex oauth
  gemini/           subscription/gemini
  coding_plan/      Kimi · GLM · MiniMax
  balance/          DeepSeek · StepFun · SiliconFlow · OpenRouter · Novita
  shared/           连通性探测、key usage 调度、官方价目表等各家共用的机制
```

`balance/` 与 `coding_plan/` 按「一族多家」而不是「一家一目录」放,因为它们本来就是
一个共享实现带一张厂商分派表(`balance.rs:36` 按 URL 认厂商)。**硬拆成五个目录
只会把一个函数切碎。**

### 决定 4:**不动 `budget_alert.rs`**,它归用量域

归位时把它挪进 `usage/`,不要因为它跟 `services/` 里的东西挨着就一起搬进 `sync/`。

---

## 5. 这个决定顺带解决/暴露的三件事

### 5.1 `quota/mod.rs` 里躺着 5 个 provider 实现

```
quota/mod.rs:101  SubscriptionQuotaCollector
quota/mod.rs:161  ClaudeChainCollector          ← provider
quota/mod.rs:344  CodingPlanQuotaCollector      ← provider
quota/mod.rs:389  ManagedCodexOAuthQuotaCollector ← provider
quota/mod.rs:451  CodexChainCollector           ← provider
```

设计 §8 说「额度采集已经是目标形状:一个接口、7 个实现、按字符串键注册」——
接口形状确实对了,**但实现放错了地方**:它们该在 `providers/` 里,
`quota/` 只该留「何时刷新、怎么退避」。

`quota/claude_quota.rs`(2,842 行)同理 —— 单个厂商的东西不该在核心模块里。

### 5.2 `ingest/` 已经按厂商分好文件了,但分在了错的目录

```
ingest/{claude,codex,gemini,opencode}.rs
ingest/session_usage{,_codex,_gemini,_opencode}.rs
```

这是 T1/T14 的成果,形状是对的(共享流水线 + 每家一个解析器)。
**但解析器属于 provider 模块**(设计 §3 的右列第四行白纸黑字写着「会话日志怎么解析」
归 provider)。归位时这些应当挪进 `providers/<厂商>/`,`ingest/` 只留流水线。

### 5.3 `model/domain.rs` 有两处硬伤,都在最底层

```rust
// 一、上行依赖(HANDOFF §15.1 已记)
use crate::usage::status::{PaceBasis, SourceClassification, UsageStatus};

// 二、provider 名字进了 model(违反设计 §4)
CodexOauth · ClaudeCli
CLAUDE_CODE_AGENT_MODULE_ID = "claude-code"
CODEX_AGENT_MODULE_ID = "codex"
```

`model` 是「谁都能依赖、它谁都不依赖」的那一层。这两处让「删掉一个 provider」
永远不可能是浅层操作。**修它们不在 T21 范围内**(需要拆文件,得单独复核),
但守卫的 allowlist 必须把它们列出来,免得被当成正常状态。

---

## 6. 本文不决定什么

- ❌ **不决定排期。** T21 写出来了但不派。
- ❌ **不决定 `model` 那两处硬伤怎么修** —— 要拆 `usage/status.rs`(1,262 行),
  单独一个任务,单独复核。
- ❌ **不决定第三个功能域(会话管理 / MCP 调度台)做不做** —— 那是产品问题。
  本文只保证:**它落地时不需要动 core**,而这个保证的依据是 §2.2 那份
  依赖清单 —— 备份同步已经做到了,说明这条路是通的。

---

## 7. 与其他文档的关系

- **修正** [`2026-08-14-modular-core-and-providers.md`](2026-08-14-modular-core-and-providers.md)
  §9.2:那张扁平图混了「功能域内部的层」与「跨域地基」两个维度。本文 §3 给出分界。
  该文 §9.2 已加注指向本文。
- **回答** [`../../HANDOFF.md`](../../HANDOFF.md) §15.2 提出的「先决定 provider 模块的
  目录形态」。
- **前置于** [`../tasks/D1-openrouter-account-balance.md`](../tasks/D1-openrouter-account-balance.md)
  与 [`../tasks/D2-packycode-account-usage.md`](../tasks/D2-packycode-account-usage.md):
  这两个任务要新增的正是 provider 模块,没有本文它们不知道该往哪放。
- **与** [`../tasks/T20-extension-seam-guards.md`](../tasks/T20-extension-seam-guards.md)
  **配套**:本文划线,T20 的守卫保证线不被磨平。§4 决定 1 明确把「谁是地基、
  谁是功能域」这个信息交给守卫承载。
