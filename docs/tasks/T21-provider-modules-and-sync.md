# T21:建 `providers/` 与 `sync/`,让 `services/` 消失

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T17 已合并**(2026-08-15 完成)。
> **形态由 [`../design/2026-08-15-provider-modules-and-feature-domains.md`](../design/2026-08-15-provider-modules-and-feature-domains.md)
> 定死,不要自己发挥。**
>
> **未排期,尚未派出。** 写在这里是为了让 D1/D2 有落点、让决定不丢。
>
> **这是纯搬运。一行逻辑都不许改。** 同 T17 的红线。

---

## 0. 背景

T17 把后端按职责重排后,`services/` 还剩 17 个文件、11,263 行。
[HANDOFF §15.2](../../HANDOFF.md) 记下:它们不属于 §9 的八个模块任何一个,
**要先决定 provider 模块的目录形态**。该决定已于 2026-08-15 定稿,见上面那份设计。

关键结论(执行前必须理解,否则会归错类):

> `services/` 剩下的是**两堆**。一堆是横向的 provider 实现(各厂商),
> 一堆是纵向的独立功能域(备份同步)。**它们的去处完全不同。**

---

## 1. 目标结构

```
src/
  model/  store/  config/  secrets/  http_client.rs   ← 地基,本任务不碰
  providers/    ← 新建
  usage/  ingest/  quota/  route/                     ← 用量监控域
  sync/         ← 新建
  api/
```

---

## 2. 搬运清单(逐文件,不要自己判断)

### 批次 1:`sync/` —— 备份同步功能域

| 从 | 到 |
| --- | --- |
| `services/s3.rs` | `sync/s3.rs` |
| `services/s3_sync.rs` | `sync/s3_sync.rs` |
| `services/s3_auto_sync.rs` | `sync/s3_auto_sync.rs` |
| `services/webdav.rs` | `sync/webdav.rs` |
| `services/webdav_sync.rs` | `sync/webdav_sync.rs` |
| `services/webdav_sync/` | `sync/webdav_sync/` |
| `services/webdav_auto_sync.rs` | `sync/webdav_auto_sync.rs` |
| `services/sync_protocol.rs` | `sync/sync_protocol.rs` |

**这一批最干净,先做。** 实测它的上行依赖只有
`store` / `error` / `config` / `http_client` / `product_identity`,
搬完之后这条依赖清单**不许变长** —— 见 §4 的自查。

### 批次 2:`providers/` —— 各厂商实现

| 从 | 到 |
| --- | --- |
| `services/subscription/mod.rs` 的共享类型 | 留在 `providers/mod.rs` |
| `services/subscription/codex.rs` | `providers/codex/subscription.rs` |
| `services/subscription/gemini.rs` | `providers/gemini/subscription.rs` |
| `services/claude_cli_auth.rs` | `providers/claude/cli_auth.rs` |
| `services/coding_plan.rs` | `providers/coding_plan/mod.rs` |
| `services/balance.rs` | `providers/balance/mod.rs` |
| `services/system_provider_connection.rs` | `providers/shared/connection.rs` |
| `services/provider_key_usage_scheduler.rs` | `providers/shared/key_usage_scheduler.rs` |
| `services/official_pricing.rs` | `providers/shared/official_pricing.rs` |

**`subscription/mod.rs` 不许整搬。** 设计 §9.3 与 HANDOFF §15.2 都点名了它:
共享类型和 `get_subscription_quota`(要发 HTTP、要碰凭据)在同一个文件里,
整搬进 `model` 会让 model 依赖 HTTP。**本任务只把它挪进 `providers/mod.rs`,
不拆** —— 拆文件是另一个任务。

### 批次 3:一个孤儿归位

| 从 | 到 |
| --- | --- |
| `services/budget_alert.rs` | `usage/budget_alert.rs` |

它是预算提醒,属用量域。**不要跟着 `sync/` 走。**

### 批次 4:删掉 `services/`

`services/mod.rs`(20 行)最后删。**这一批做完 `services/` 必须不复存在。**

---

## 3. 明确**不**在本任务范围内

下面三件事设计文档 §5 都点名了,**它们是后续任务,本次一个字都不许动**:

- ❌ `quota/mod.rs` 里那 5 个 provider collector 迁进 `providers/`
- ❌ `quota/claude_quota.rs`(2,842 行)迁进 `providers/claude/`
- ❌ `ingest/{claude,codex,gemini,opencode}.rs` 那批解析器迁进 `providers/`
- ❌ `model/domain.rs` 那两处硬伤(上行依赖 + provider 名字)

理由:本次已经动了 17 个文件,再叠上去就做成了没法复核的巨型 diff。
**T17 的教训是分批,不是分一次。**

其余照 T17 §4:不改逻辑、不重命名、不合并拆分文件、不动 `src/`(前端)、不加依赖。

---

## 4. 依赖方向自查(搬完必须跑)

```bash
# sync/ 不许依赖任何功能域内部的层
grep -rn "use crate::\(usage\|quota\|route\|ingest\|api\)" src-tauri/src/sync/

# providers/ 不许依赖用量域的业务层(它是被调用方)
grep -rn "use crate::\(usage\|quota\|route\|api\)" src-tauri/src/providers/
```

**第一条输出必须为空。** 第二条若有输出,逐条在报告里解释 —— 允许存在
(例如 collector 要用 `quota` 里的接口定义),但每一条都要能说清为什么。

---

## 5. 验收判据(用 T17 学到的那条,不要用旧的)

[HANDOFF §15.3](../../HANDOFF.md) 记下的教训:

> 「那条 diff 命令输出必须为空」对模块搬家**不现实** —— 内联全限定路径必须改,
> 路径变长会触发 rustfmt 重新折行。T17 实际输出 520 行。
> **更好的机械判据是比对字符串字面量**:纯搬运不该改动任何字符串。

所以本任务用这条:

```bash
# 搬运前后,全树字符串字面量的数量与内容应当一致
# (唯一允许的差异是含模块路径的测试定位串)
```

报告里给出前后数量对照,以及每一条差异的解释。

---

## 6. 完成的标准

- 四个批次各自一个提交,每批之后六项检查全绿
- `services/` 目录不复存在
- §4 两条自查:第一条为空,第二条每行有解释
- §5 字面量比对:差异逐条解释
- 全部测试通过且断言一字未改
- 报告里给出:搬运前后各目录的文件数与行数对照表
