# LLM Usage Bar — 全局交接文档(合并版)

最后核实:2026-08-07(所有事实当天用 git / gh / sqlite3 逐条验证过,不是抄旧文档)
最后更新:2026-08-07 傍晚 —— Claude 重置时间线告一段落,见 §9(新增,下周继续)

> **这是唯一的交接文档。** 它取代并吸收了以下分散文档,那些文件不要再单独更新:
>
> | 被取代的文档 | 原位置 | 状态 |
> | --- | --- | --- |
> | `docs/task-state/current-task.md`(2026-07 版) | 主仓库 | 历史,PR #6 时代,已完成 |
> | `docs/task-state/2026-07-12-dependabot-ci-failures-handoff.md` | 主仓库 | 作废 —— PR #4/#5 已被关闭重开为 #15/#18 |
> | 2026-08-07 checkpoint(`docs/task-state/current-task.md` 更新版) | 分支 `claude/llm-usage-monitoring-app-4a9554` 提交 `67676ef9e` | 内容已并入本文 §2、§6、§7 |
> | `HANDOFF.md` 报告(一)(二) | worktree `usage-model-agent-classification-03acf1`(报告二未提交) | 内容已并入本文 §4、§5 |
>
> 交接方式:后续 agent 把新发现**追加到本文件**,不要另开新文档。

---

## 0. 开工前必做的三步(两次翻车都是因为跳过了这里)

1. **基线就是 `main`。** 2026-08-07 傍晚实测:本地 `main` = `origin/main` =
   `d21d3d682`,`SCHEMA_VERSION = 24`。之前"本地 main 落后 55 个提交"的问题
   **已经解决**,不要再从别的分支拉线。

   ```bash
   git fetch origin && git log --oneline -1 origin/main
   ```

   > 别照抄这里的哈希 —— 上面这条命令的输出才算数。`main` 与 `origin/main`
   > 当前一致(0 个未推)。未合并的活线有两条,且是父子关系:`claude/quota-reset-latch`
   > (PR #23,见 §9.2)和建在它之上的 `feat/provider-api-keys`(PR #24,见 §10)。
   > 5 个陈旧本地分支已于 2026-08-11 核实并删除,详见 §9.5。

2. **动数据库/想本地跑 app 之前,先对版本。** 生产库 2026-08-07 傍晚实测 v24:

   ```bash
   sqlite3 ~/.llm-usage-bar/llm-usage-bar.db "PRAGMA user_version;"
   ```

   分支的 `SCHEMA_VERSION`(`src-tauri/src/database/mod.rs:57`)必须 ≥ 库版本,
   否则 app 启动即崩:`authoritative database schema v24 is outside supported range …`。

   > **v25 和 v26 已经被 `feat/provider-api-keys`(PR #24)占用了**,别再往那两个号上编。
   > 从 main 拉新线的话下一个是 v25;从 PR #24 之后拉的话是 **v27**。编号冲突正是 8 月两
   > 次翻车的直接死因,动手前先跑一遍上面那两条命令确认自己的基点。

3. **读完本文件再动手。** 8 月的两条 feature 线全栽在同一个坑
   (基点过时 → 迁移编号错 → 装上就崩),第二次翻车时答案已经写在交接文档里了。

---

## 1. 现状总览(2026-08-07 逐条核实)

### 生产环境:健康

- 已安装 app:`/Applications/LLM Usage Bar.app`(3.16.5,v24)。
  **注意:§9.2 的新构建已生成但尚未安装**,装着的这份不含重置时刻锁存。
- 生产库:`~/.llm-usage-bar/llm-usage-bar.db`,**v24**,81 MB,47.8k 条 `usage_events`,
  `PRAGMA integrity_check` = ok,数据正常增长。**数据库从未损坏过** ——
  8 月的两次"崩溃"都是版本上限检查拒绝打开旧代码,不是写坏。
- 备份:`~/.llm-usage-bar/backups/`(v19 起各阶段都有,最新
  `db_backup_20260810_171554.db`)。
- 日志:`~/.llm-usage-bar/logs/llm-usage-bar.log`;崩溃日志 `~/.llm-usage-bar/crash.log`。

### 各条线的状态

| 线 | 分支 / 位置 | 状态 | 需要行动? |
| --- | --- | --- | --- |
| 主线 | `main` = `d21d3d682` | 绿,**v24**,与 `origin/main` 同步(0 个未推) | 否 |
| Claude 额度重置时间 | `claude/quota-reset-latch`(已推,PR #23) | 锁存已实现并验证;CI 全绿、`MERGEABLE`/`CLEAN`;`/usage` 探测待做 | **等合并**,见 §9 |
| Provider 多 key 花费 | `feat/provider-api-keys`(已推,PR #24) | 建在 PR #23 之上,自己 3 个提交;**SCHEMA_VERSION 24 → 26**(两个迁移,各带 validator);本机全套验证绿(见 §10) | **先合 #23,再合 #24**;目视验证未做 |
| 用量按模型/Agent 分类 | **✅ 已搬上 main(2026-08-07,提交 `e23894168`)** | Codex(max)在隔离 worktree 移植,Claude 逐 hunk 复核并独立重跑全套验证(Rust 1039/0、tsc、prettier、59+9 前端测试全绿) | 旧分支 `claude/usage-model-agent-classification-03acf1` 及其 worktree 已作废,可删(需 `-D`);目视验证仍欠 → P4 |
| 红绿灯燃烧速度投影 | **✅ 已搬上 main(2026-08-07,提交 `bb8514def`,迁移重编号 v23→v24)** | Codex(max)移植 + 签名脚本修复一并带上;Claude 复核(DDL 范围、预测行无机密、阈值为常量)并独立重验(Rust 1066/0、前端 192+9 全绿) | 旧分支 `claude/traffic-light-logic-redesign-3fbc8e` 及 worktree 可删;**注意:新代码 SCHEMA_VERSION=24,装上后旧 3.16.5 打不开升级后的库,须一步到位** |
| 2026-08-07 checkpoint 文档 | `claude/llm-usage-monitoring-app-4a9554`(= main 的内容 + 1 个 docs 提交 `67676ef9e`) | 纯文档分支,内容已并入本文 | 可删分支和 worktree |
| 本合并任务 | `claude/consolidate-error-issues-4d8721` | 即本文件所在分支 | 合并进 main 让后续 agent 能看到 |
| 6 个 `codex/*` 旧线(7 月) | ~~`.worktrees/`~~ | **✅ 已清理(2026-08-07)**:6 个 worktree、6 个分支、3 个失效 bridge worktree 全部移除(删前核实 0 独有提交、工作区干净) | 否 |

> 更正一个旧文档里的错误结论:报告(二)说
> `claude/llm-usage-monitoring-app-4a9554`"不是 origin/main 的祖先,是另一条线"。
> 实测其父提交 `e337dd32e` **就是 main 的祖先**,该分支只是 main + 一个文档提交,
> 没有失散的代码。

### GitHub PR(2026-08-11 实查)

| PR | 内容 | 检查 | 行动 |
| --- | --- | --- | --- |
| #24 | Provider 多 key + 每把 key 的花费 | 本机全套绿;CI 待跑 | 合在 #23 之后 |
| #23 | Claude 额度重置时刻锁存 | 绿(Frontend/Backend Checks 均 SUCCESS) | **可以合,先合这个** |
| #22 | frontend-deps 依赖组(40 项) | 待查 | 未处理 |
| #21 | cargo-deps `base64` 0.23.0→0.23.1 | 待查 | 未处理 |

上一版本节列的 #12–#18 已全部合并或关闭,不再有效。7 月文档里的 PR #4/#5 巨型
Dependabot PR 同样已不存在。它们建议的"Dependabot 分组限制 minor/patch"策略仍未落实
(见 §3 P5)—— #22 又是一个 40 项的大组,同一个问题还在复发。

---

## 2. 已经解决的问题(列在这里防止重查)

- **main 一度全红、5 个 Dependabot PR 全红** —— 单一根因:开发机是 macOS,CI 是
  ubuntu-22.04,任何 `cfg(target_os)` 形状的缺陷本地编译不到,只有 CI 的
  `clippy -D warnings` 能抓到。修于 `47edac944`。Dependabot PR 只是被株连。
  → 教训:动了平台门控代码,就要靠通读把整类问题审完,并把 CI 当成真正的门。
- **clippy `truncate` 告警**(`claude_quota.rs`,advisory lock 文件)——
  main 上已用 `.truncate(false)` 正确修复(`claude_quota.rs:416`)。
  报告(一)§5.4 的"需确认"可以销掉。
- **本地 `main` 落后 55 个提交** —— 已更新到 `f3aadadec`,与 origin 一致。
- **PR #17 的 8 条自动 review findings** —— 7 条真问题修于 `e337dd32e`;
  1 条(称 v21 删了 `proxy_request_logs`)是 reviewer 错了。其中 3 条涉及已迁移
  数据,在本机 v21/v22 备份和 v23 现库上逐一核实为空集,**不需要修复迁移**;
  换一台机器的库要重新验证再下结论。
- **PR #6 时代的全部实现、review 与 CI 修复**(2026-07-12 checkpoint 的全部内容)——
  完成并进入 main,细节留在 git 历史与旧文档,不再复述。
- **v19 数据在 v22 成本回填后的真实性验证** —— 41,599 条事件保留,30 天口径从
  $112.98 修正为 $3,346.14,与独立工具 ~$3,000 相符。

---

## 3. 待办问题清单(按优先级)

### P0 — 把两个 PR 合上 main(2026-08-11 新增,顺序不能反)

两条线是父子关系,`feat/provider-api-keys` 的前 5 个提交**就是** PR #23 的那 5 个:

1. **先合 PR #23**,并且**必须用 merge commit**(仓库三种方式都开着)。squash 或 rebase
   会重写那 5 个提交的哈希,#24 仍带着原版,同一份改动就会以两组提交的形式出现在 main
   的祖先里 —— 合 #24 时冲突或 diff 全乱。main 本来就用 merge commit(#17 即是),
   这不算破坏惯例。
2. 再合 **PR #24**。#23 一合,#24 的 diff 会自动收敛成它自己的 3 个提交。

两个都合完之后,`SCHEMA_VERSION` 到 v26,新迁移从 **v27** 起编。

### ~~P1 — 把「分类」线搬上 main~~ ✅ 已完成(2026-08-07,提交 `e23894168`)

`80d791725` 已移植到 main。移植中的语义决定:去重 SQL 与 main 现有谓词逐字
一致(独立比对 543 字节);无归属桶改为**无条件**排最后(原实现只在平局时);
新测试 helper 补上了 main 新增的 `UsageEvent.pricing_origin`;manifest 重锚了
12 条 `lineNumber`。§4 的搬迁指南保留作历史参考,语义约定一节仍是有效契约。

### ~~P2 — 把「红绿灯」线搬上 main~~ ✅ 全部完成(2026-08-07,提交 `bb8514def`)

功能与签名脚本修复(`5ed753e08`)都已在 main。移植中的关键适配:迁移重编号
v23→v24;WebDAV/S3 后端同步在 main 上仍在,预测表保留排除;`UsageLightInfo`
改用 main 的 `SettingsSection` 外壳;headroom 阈值(115%/85%)落为编译期常量
而非 settings.json 可覆盖项。**main 现在是 SCHEMA_VERSION=24**:下一次构建
安装的 app 首启会把生产库 v23→v24(自动做迁移前备份),之后旧 3.16.5 无法再
打开该库,构建安装要一步到位。

### ~~P3 — PR #18:`node-pty` 构建脚本放行~~ ✅ 已解决(2026-08-07,用户拍板)

用户选择**关闭 #18**而非放行 `node-pty` 的 postinstall。分组策略已限为
minor/patch(见 P5),Dependabot 下个周期会重建一个不含 major 的小 PR,
大概率不再牵涉 `node-pty`。若重建的 PR 仍要求放行某个传递依赖的构建脚本,
那依旧是供应链决策 —— **问用户,不要默默加允许列表**。

### P4 — 目视验证 ⏳ 只剩最后一眼(2026-08-07)

新版已通过 `script/build_and_run.sh` 构建、签名、安装并正常运行;生产库已
迁移 v23→v24(迁移前备份 `db_backup_20260807_134610.db`),日志健康,
session 同步正常。**剩下的只是用户亲眼扫一遍**:三个 Tab(Providers /
Models / Agents)的渲染、新红绿灯与 pace 详情、暗色/亮色、窄窗口。
测试全绿但像素没人看过 —— 发现视觉问题记回本文件。

> 2026-08-11 补充:P0 的两个 PR 合完之后会有一个新构建,那一次可以把这里欠的一眼、
> §9.2 的重置时间显示、§10 的多 key 卡片一起看掉,不必分三次装。

### P5 — 低优先级 / 观察项

- **合并绿的 #12–#15**:仍开着 —— agent 侧被权限分类器拦截(`gh pr merge` 属
  对外操作),留给用户执行:
  `for n in 12 13 14 15; do gh pr merge $n --squash --delete-branch; done`
- ~~Dependabot 分组策略~~ **✅ 已落实(2026-08-07)**:两个组各加
  `update-types: [minor, patch]`(组名未动,避免现有 PR 被重建);major 此后
  单独成 PR。注意:#18 在下个更新周期可能被 Dependabot 按新规则重建,
  重建后的 minor/patch 组 PR 未必再碰 `node-pty`,§P3 的决策可能因此消失。
- **flaky 前端测试**:`UsageDashboardPage.test.tsx` → "queries exact Provider-wide
  ranges…" 全量跑时偶发超时(~6.8s),单跑必过。早于近期改动,CI 未见失败,未查。
- **`planRenewsAt` 永远为空**:refresh 签发的 `id_token` 疑似只带
  `chatgpt_plan_type` 不带订阅日期;app 有意不持久化 `id_token`,无法回看确认。
  现象:续订 tooltip 不渲染。无害。
- **2026-08-07 10:17 一次性启动崩溃**,同一 app 10:18 重启即好,独立偶发,未复现。
- ~~清理垃圾~~ **✅ 已完成(2026-08-07)**:坏 app 残留已进废纸篓(Finder 删除,
  可恢复);6 个 `.worktrees/` 旧 worktree + `codex/*` 分支已删(`git branch -d`
  逐一确认已并入 main);3 个 bridge worktree 已 prune。
  仅剩 `claude/llm-usage-monitoring-app-4a9554` 分支/worktree(内容已并入本文,
  但其提交 `67676ef9e` 未进 main,删除需 `-D` 强删,留给用户决定)。

---

## 4. 搬迁指南:用量按「模型」和「Agent」分类

**做了什么**:用量面板顶部三个并列 Tab(Providers / Models / Agents),共用时间
范围选择器。后端两个新命令 `get_model_usage_dashboard` / `get_agent_usage_breakdown`
(`src-tauri/src/usage/aggregation.rs` 新增两个聚合函数 + `domain.rs` 7 个视图结构);
前端新增 `ModelUsagePage.tsx`、`AgentBreakdownPage.tsx` 等 8 个文件,
改 `UsageDashboardPage.tsx` 等 6 个文件,四语言各 +31 条文案。

**语义约定**(与既有 Provider 面板一致,review 时按这些点检查):
半开区间 `[start, end)`;只统计 `enabled = 1`;proxy↔session_log 链接去重;
全部无价返回 `None` 而非 `"0"`;空 model id 归一 `"unknown"`;排序后端定死;
Agent 维度含已归档与无归属桶(`agentModuleId: null` 永远最后)。

**步骤**:

1. 改动在本地提交 `80d791725`(worktree
   `.claude/worktrees/usage-model-agent-classification-03acf1`,未推送)。
   worktree 里还有一处未提交:`HANDOFF.md` 的报告(二)部分 —— 内容已并入本文,
   可以不要。
2. 从 `main` 拉新分支,把 `80d791725` 搬过去。两份旧报告在 rebase 还是重放上分歧,
   **建议 `git cherry-pick 80d791725` 后一次性面对完整冲突**,与报告(二)的结论
   一致:比在 rebase 中途逐个解可控。
3. 已知冲突热点(main 侧 PR #17 在同区域 38 文件 +3563/−1633):
   - `AgentUsagePage.tsx`、`DashboardModuleSwitcher.tsx`、
     `useDashboardModuleSelection.ts` 在 main 上**已删除** —— 本改动不依赖,无冲突;
   - `UsageDashboardPage.tsx`(main 只改 2 行,冲突面小)、
     `src/types/usageDashboard.ts`、`src/lib/query/usageDashboard.ts` 要手工并;
   - 核对仍然存在且签名未变:`usageDashboardProjection.ts` 的 `MeteredCostStatus`、
     `addDecimalStrings`;`usagePresentation.ts` 的 `formatTokensCompact`、
     `dashboardProviderIcon`;
   - 后端核对:去重 SQL 常量 `NOT_A_LINKED_SESSION_DUPLICATE` 的文本是否仍与
     main 的既有子查询逐字一致;`billing_kind` 取值;`agent_modules` 的
     `name`/`archived_at`/`visible` 列。
4. 重锚 `tests/config/productIdentityCompatibilityManifest.json`(i18n 插块使行号
   下移;按 `context` 字符串在新文件重新定位,重复项按原顺序取最近匹配)。
5. 重跑全套验证(§7 的命令),再补 P4 的目视验证。

此改动**不含迁移**,没有 SCHEMA_VERSION 问题,搬完即可跑。

---

## 5. 搬迁指南:红绿灯按燃烧速度投影

**做了什么**:菜单栏红绿灯从「剩余额度静态阈值」改为
`headroom = 剩余额度 / (基准速率 × 有效剩余时间)`,≥1.15 绿 / <0.85 红 / 之间黄
(黄区是误差棒,不是设置项)。三层速率来源逐级回退(快照差分 → 窗口均速 →
无 `resets_at` 时退回旧静态阈值);按「星期几×小时」作息加权,**所有强度为 1 时
逐字节退化为不加权判定**(结构性保证 + 等值测试);新表 `usage_light_predictions`
只写不读做校准日志;顺带修了 `project_subscription` 把 5h/7d 两窗口 `used` 取
max 混判的真 bug(改为各自判定取最差)。

**步骤**:

1. 改动已推送:`origin/claude/traffic-light-logic-redesign-3fbc8e`,提交
   `45898d3ba`(44 文件 +4339/−543)+ 签名脚本修复 `5ed753e08`(已于
   2026-08-07 补提交推送,抢救完成)。
2. 从 `main` 拉新分支重放 `45898d3ba` 和 `5ed753e08`。报告(二)明确建议
   **重新落一个提交而非 rebase**。
4. **迁移重编号:v19→v20 改成 v23→v24**,涉及 `database/mod.rs` 的
   `SCHEMA_VERSION`、`schema.rs` 迁移分发、`usage_light_prediction_migration.rs`
   函数名与 `validate_schema_v*_complete`。
5. 重点冲突:main 侧对 `src-tauri/src/usage/tray_snapshot.rs` 有实质改动,与本次
   大改同文件,逐 hunk 判断文本冲突还是语义冲突。
6. 构建安装前:`sqlite3 ~/.llm-usage-bar/llm-usage-bar.db "PRAGMA user_version;"`
   必须 ≤ 新分支 `SCHEMA_VERSION`。旧基线上的验证结果(Rust 2592 过、前端 669 过)
   搬迁后作废,需重跑。

---

## 6. 不可回退的 invariants(每条都对应一个真实翻过的 bug)

- **`usage_events` 只追加**,有不可变触发器。要改写成本的迁移必须像
  `usage/cost_backfill_migration.rs` 那样在 savepoint 里先删后建触发器。
- **无价 ≠ 免费。** 查不到可用价格必须显示"cost unavailable",绝不能是 `$0`;
  反过来,窗口内**没有用量**就是确定的 `$0`,不是 missing。两个方向都出过 bug。
- **缓存语义按厂商区分。** OpenAI/Gemini 的 `input_tokens` **含** cache read,
  Anthropic 是净值。Codex 流量 ~96% 是 cache,搞反过一次,成本虚高 ~65 倍。
  见 `usage/metering/calculator.rs` 的 `input_includes_cache_read`。
- **配额载荷必须脱敏。** `QuotaSnapshot::raw_payload` 是完整序列化的
  `SubscriptionQuota`;对外只提升具名字段,照 `QuotaStatusView::from_snapshot` 做。
  `usage/dashboard.rs` 有测试把关。
- **绝不记录/持久化 `id_token`、access token、refresh token。**
  只有派生的非机密值(套餐类型、续订时间)能跨边界。
- **Provider 的 `enabled` 意为"在用户列表上"**,不是"配置完整"。
  停用只隐藏,历史保留。
- 自定义 model id 在**写入和读取两侧**都过 `clean_model_id_for_pricing` 归一;
  family 价格行覆盖带日期变体,反之不成立。
- 产品方向(2026-08-07 起):只回答两个问题 —— 订阅额度还剩多少、用量值多少钱。
  schema v21 删掉的九张表对应的功能(**proxy 及其请求日志、MCP 管理、prompt 管理、
  provider 切换、按 provider 配置起终端**)不要复活,残留引用当死代码删。

---

## 7. 本机工作约定

- **Rust/Tauri 一律走包装器**:`pnpm rust -- <cargo args> --manifest-path
  src-tauri/Cargo.toml`、`pnpm tauri -- …`/`pnpm dev`。AGENTS.md 禁止裸 `cargo`;
  包装器不 `cd`,所以 `--manifest-path` 必带。删 worktree 前跑
  `pnpm cargo:cache -- status`。
- **聚焦跑前端测试**:`pnpm test:unit <path>`,pnpm 11 下**不要**在路径前加 `--`。
- **完整验证门**(两条搬迁线都要过):
  Rust 全量测试、clippy `-D warnings`、`fmt --check`、`pnpm typecheck`、
  `pnpm format:check`、前端单测、`git diff --check`。
- **manifest 重锚**:`tests/config/productIdentityCompatibilityManifest.json` 按
  `file:line:column` 钉住遗留 `cc-switch` 字面量,任何挪动行号的编辑之后都要重锚
  (按 `context` 匹配;一文件多条同 context 的按文件序配对)。
- **CI 特性**:Backend Checks ~13 分钟;出现过被基础设施中途取消 ——
  "The operation was canceled" 且无测试输出**不是代码错误**,先重跑再查。
- **cfg 陷阱**:开发机 macOS、CI ubuntu。平台门控代码的错误本地不可见,
  只有 CI 能抓(§2 第一条的根因)。
- **签名**:`~/Library/Keychains/llm-usage-bar-signing.keychain-db` 是独立密码
  且用户没有密码。`script/build_and_run.sh` 的修复(提交 `5ed753e08`,在
  traffic-light 分支上,搬迁时一并带上):只在提供了
  `LLM_USAGE_BAR_SIGNING_KEYCHAIN_PASSWORD` 时才用专用钥匙串,否则 login 钥匙串;
  真签探针带 5 秒看门狗(codesign 对锁住的钥匙串会弹 GUI 框无限等待,不会快速
  失败);回落身份 `LLM Usage Bar Local Development`。
  **不要对该钥匙串跑 `security show-keychain-info` / `find-certificate`** ——
  会当着用户弹一个没人知道密码的输入框。
- **别对着真实数据目录乱跑 dev**:先确认 schema 匹配(§0 第 2 步),先退掉已装
  app(`tauri-plugin-single-instance` 会把启动交接给已在运行的实例),`pnpm dev`
  放前台。
- **委派复核纪律**(历次沿用):子 agent 的自述不算数 —— Codex 曾自报 15 个测试
  失败,实为其沙箱限制,本机重跑全绿;也曾有 agent 用 `head` 截断测试输出漏看
  26 项失败。必须自己看真实 diff、自己完整跑验证。

---

## 8. 仍然有效的参考文档(未被本文取代)

- `docs/superpowers/specs/2026-07-17-provider-only-monitoring-design.md` —— 产品方向定调
- `docs/superpowers/specs/2026-08-03-official-pricing-refresh-design.md` —— 官方价目刷新(已实现)
- `docs/superpowers/specs/2026-08-05-custom-pricing-usability-design.md` —— 自定义价格可用性(已实现,`317911b33` / `08dcb9dea`)
- `AGENTS.md` —— 包装器与 Kimi 委派规则
- `docs/usage-dashboard-acceptance.md` —— 验收 runbook(PR #6 时代,流程仍可参考)
- 其余 `docs/superpowers/plans/*` 与 `design-qa.md` 为历史实施记录,只作考古用

---

## 9. Claude 订阅额度的「重置时间」线(2026-08-07 傍晚,未完待续)

起因:用户发现 Claude 的订阅额度不显示重置时间,而 Codex 的显示正常。

### 9.1 根因(全部实测,非推断)

不是「拿不到」,是「**保不住**」。Claude 的额度由两个本地源拼成:

| 源 | 文件 | 百分比 | 重置时间 | 刷新 |
| --- | --- | --- | --- | --- |
| Claude Desktop | `~/Library/Application Support/Claude/plan-usage-history.json` | ✅ | ❌ **永远没有** | 每 15 分钟,全自动 |
| Claude Code statusline 桥 | `~/.llm-usage-bar/runtime/claude-statusline-quota.json` | ✅ | ✅ | 仅终端 TUI 渲染时 |

`collect_local_quota_from_paths_at` 按「谁的 `observed_at` 新用谁」选源,且刻意
禁止跨源拼接。于是每 15 分钟自动刷新的 Desktop **必然反超**偶发的 statusline,
重置时间随之消失。实测存活 **9 分钟**:

```
14:50:10  reset=2026-08-07T10:00:00+00:00   ← statusline(14:49:35)赢
14:59:34  reset=同上
          --- Desktop 采样 15:01:26 落地 ---
15:04:40  reset=(空)  fh 跳到 18%          ← Desktop 反超
```

生产库佐证:`quota_snapshots` 里 `system-claude-subscription` 自 2026-07-30 建库
起 **989 条,0 条**带过重置时间。不是回归,是从来没通过。

**桌面版为什么零出口**(四条通道逐一排除,别再重查):

- `statusLine` 命令只在终端 TUI 的 React 渲染树里 spawn(`nRT` 是个用
  `ix.useRef` / zustand selector 的 hook)。桌面版 UI 不挂载那棵树 →
  命令永不执行。实测:桌面版 session 存储实时在写,statusline 缓存 13h40m 未动。
- hooks 在桌面版**能跑**(用户的 `dcg` PreToolUse 钩子正常拦截),但公共 payload
  只有 `session_id / transcript_path / cwd / prompt_id / permission_mode /
  agent_id / agent_type / effort`,**不带额度**。
- OTel 20 个 `claude_code.*` 指标只有 `cost.usage` / `token.usage`,无额度窗口。
- session transcript 不记额度:扫 8 个会话 5,079 行,结构化命中 **0**
  (关键词命中全是对话正文里打的字,别被 `grep -c` 骗了)。
- Claude Desktop 自己的 Local Storage / IndexedDB / Session Storage 搜
  `resets_at`、`five_hour` 全 0;`plan-usage-history.json` 1421 个样本的字段
  union 就是 `{t, org, u:{fh, sd}}`,**没有重置字段这个概念**。

额度只活在进程内存里:`BLu(e)` 直接读 `anthropic-ratelimit-unified-*` **响应头**,
`~/.claude` 下无任何文件持久化它。

### 9.2 已完成并提交:重置时刻锁存

分支 `claude/quota-reset-latch`(**未推远端、未合 main、未开 PR**):

```
4e7f8fa5a  feat(usage): latch Claude's quota reset instant so it survives the Desktop source
4b2802eb5  fix(test): exclude nested .claude worktrees from vitest collection
```

思路:重置时刻是**固定墙钟**,不随用量变化,所以不必跟着百分比走。锁存在
`~/.llm-usage-bar/runtime/claude-quota-reset-latch.json`(0600 / 独立 flock /
**无迁移,schema 仍 v24**),Desktop 赢时由锁存供重置时间。

**同源判据(踩过坑,别改回去)**:两源无共同账号标识,所以跨源携带必须有正证据。
两个窗口漂移速度差一个量级 —— 实测 14:01→15:46,`fh` 5%→35%,`sd` 63%→66%。
Desktop 每 15 分钟才采一次,所以:

- **7 天窗口定身份**(容差 2 点),两个无关账号在这个数上撞车很难;
- **5 小时窗口只否决**:较旧读数高于较新读数 → 单账号窗口内不可能;
- 任一矛盾 → 否决整趟**并清空全部锁存**(身份是两源之间的性质);
- 百分比跌破锁存时的值 → 窗口已滚过 → 丢弃锁存。

> **这条最初写错过。** 第一版用「两源百分比近似相等」当判据,34 条单元测试全绿,
> 一碰真实数据就崩:Desktop 滞后 15 分钟本身就能差 6 点,被判成异账号,不但锁不上
> 还**主动擦掉**已有锁存。合成数据的时间戳是随手编的,测不出这个。

验证:`cargo test` 1079 通过、`clippy -D warnings` 干净、`cargo fmt` 干净、
`vitest` 333 通过、`tsc` 干净;外加一次针对**真实文件**的临时探针(用完已删):
statusline 缓存拿掉、只剩 Desktop 时,重置时刻仍解析得出。

前端同时改了:有百分比但无重置时间时,仪表盘不再整行塌掉(会跳高度)、托盘不再
渲染 `Resets —`;`formatResetTime` 新增 `unreported` 区分「源不提供」与「即将重置」。
4 个语言包各加 `usageDashboard.resetTimeUnknown` / `trayUsage.resetTimeUnknown`。

**构建产物已生成但未安装**:`release/tauri-target/release/bundle/macos/LLM Usage Bar.app`
(3.16.5,已签名 `valid on disk`,含 `claude-quota-reset-latch` 字符串)。
`/Applications` 里仍是旧构建。装的时候:

```bash
cp -R "/Applications/LLM Usage Bar.app" "/Applications/LLM Usage Bar.app.bak-v24" \
  && osascript -e 'quit app "LLM Usage Bar"' \
  && rsync -a --delete "/Users/max/LLM Usage Bar/release/tauri-target/release/bundle/macos/LLM Usage Bar.app/" "/Applications/LLM Usage Bar.app/" \
  && open -a "LLM Usage Bar"
```

schema 仍 v24,新旧构建都能开同一个库,可回退(和上次 v23→v24 那种一步到位不同)。

**验收顺序**(第三步才是真正的验收点):

1. 装上打开 → 应显示「未提供重置时间」,而不是整行消失
2. 开一次终端 `claude` 发句话 → 重置时间出现
3. **等 15 分钟以上再看 → 应该还在**(旧行为是 9 分钟后消失)

### 9.3 下周要做的:`/usage` PTY 探测(用户已拍板,尚未实现)

参考 [steipete/CodexBar](https://github.com/steipete/CodexBar)([docs/claude.md](https://github.com/steipete/CodexBar/blob/main/docs/claude.md))。
它读 Claude 用五条路,与本项目相关的是两条:

- **首选 `GET https://api.anthropic.com/api/oauth/usage`**,带
  `Authorization: Bearer <token>` + `anthropic-beta: oauth-2025-04-20`,token 取自
  `~/.claude/.credentials.json` 或 Keychain `Claude Code-credentials`,需
  `user:profile` scope。直接返回两个窗口含重置时间。
- **兜底 CLI PTY**:起 `claude`、发 `/usage`、剥 ANSI 解析面板。

**用户选了 PTY 路线(方案 B),明确不走 OAuth 接口** —— 因为那会让本 app 从
「零网络、纯读本地文件」变成「拿用户 OAuth token 打未公开接口」,是定位变更。
(CodexBar 这么做属通行做法的证据,但不构成 Anthropic 许可的证明。)

**关键实测结论**:`/usage` **完全免费**。同屏显示
`Total cost: $0.0000` / `Total duration (API): 0s` /
`Usage: 0 input, 0 output, 0 cache read, 0 cache write`,却拿到:

```
Current session
███████████████████████████▌  55% used
Resets 6pm (Asia/Singapore)
Current week (all models)
█████████████████████████████████▌  67% used
Resets Aug 11 at 6pm (Asia/Singapore)
```

与锁存里存的值**逐位相同**,是来自 Claude Code 自身面板的独立交叉验证。

> **本轮曾给出过一个错误结论:「必须真发一次消息才能拿到额度」。** 那次探针只把
> TUI 起起来干等 30 秒,**从头到尾没发过 `/usage`**。由此测出的「一次极简 turn
> 要 50,122 input token」是真的,但它回答的是「发一句话多少钱」,不是「取额度多少钱」
> —— 后者是 0。别再拿那个 5 万的数字论证「取数太贵」。

**实现要点与已踩的坑**:

- 命令:`claude --mcp-config <空配置> --strict-mcp-config`,
  env `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`。空 MCP 配置内容 `{"mcpServers":{}}`。
- **不要传 `--allowed-tools ""`** —— 传了空字符串那次面板不渲染。已验证:
  换成不传即正常。(曾误判成「新目录信任提示拦截」,**实测全新目录照样出面板**,
  信任提示不是问题。)
- PTY 必须给窗口尺寸(`TIOCSWINSZ`,实测 45×130 可用),否则 TUI 不布局;
  stdin 不能接 `/dev/null`,否则子进程立刻 EOF 退出(`script -q /dev/null claude`
  这种写法起不来,前两次探针就是这样白跑的)。
- 时序:启动约 12–14 秒后再发 `/usage`,面板在其后数秒内渲染完;总预算
  ≤45 秒并强制 kill(`SIGTERM` 再 `SIGKILL`)。实测无残留进程,
  且因继承了 `CLAUDE_CODE_CHILD_SESSION` 标记连 transcript 都不落盘。
- **`/usage` 不填 statusline 读的那个 store**(缓存 mtime 不变),所以桥抓不到,
  必须自己解析屏幕文字。
- 解析目标:`Current session` / `Current week (all models)` 两个表头下的
  `NN% used` 与 `Resets <文本>`。**难点是重置文本**:`6pm (Asia/Singapore)` 与
  `Aug 11 at 6pm (Asia/Singapore)` 是人类可读格式,要转成绝对时刻,依赖时区与
  「今天的 6pm 是否已过」的判断。CLI 版本变了格式可能变。
- 建议接法:探测产出的东西与 statusline 观测同形(百分比 + 重置时刻),直接复用
  现有 §9.2 的锁存与同源判据链路,不要另起一套。探测**不要**在 5 分钟轮询里同步跑
  (要起进程、约 20 秒),应按需触发 + 限流,失败时静默降级到锁存。
- 实现档次:纯后端 Rust(PTY、进程生命周期、超时兜底、ANSI 剥离、时区换算),
  按本机约定该派 Codex,建议 `max` 或 `ultra` + 后台。**注意 `~/.codex/config.toml`
  的 `service_tier` 是 `default` 而非 `priority`,要 Fast 必须显式传
  `--service-tier priority`。**

### 9.4 顺带修掉的既有 bug

- **`vitest.config.ts` 排除漏洞**(已提交 `4b2802eb5`):排除列表写的是
  `**/.worktrees/**`,但 agent 分支的 worktree 在 `.claude/worktrees/`,路径不匹配。
  嵌套 worktree 自带 node_modules → 加载第二份 React → 每个 render 都死在
  null dispatcher → `npx vitest run` 平白多出 **41 个假失败**。已补
  `**/.claude/worktrees/**`。
- **`productIdentityCompatibilityManifest.json` 钉死行号**:往 `en.json` 插 key
  会推移行号,导致 `productIdentity.test.ts` 失败。修法是**按内容重新锚定**
  (找 `context` 原文所在行,就近取),不要加固定偏移量 —— 那样原来钉错了会静默烂掉。
  本次 12 条各 +1。
- **`.claude/worktrees/consolidate-error-issues-4d8721` 已删**(§P5 遗留的漏网):
  删前核实工作区干净、`main..该分支` 为空、两者同指 `d21d3d682`;用
  `git worktree remove` + `git branch -d`,元数据一并清理。

### 9.5 状态提醒:本机未推远端的东西

> 本节修正过一次。初稿写的是「main 领先 origin/main 一大截」,**那是错的** ——
> `git fetch` 后实测 `git rev-list --count origin/main..main` = **0**,main 早已
> 推干净。当时只看了 `git log --all --not --remotes=origin` 的总数就下了结论,
> 没查那些提交挂在哪个 ref 上。

`--all` 会把 stash、旧 tag、陈旧分支全算进去。实际分布(2026-08-07 傍晚实测):

- **`main`:0 个未推**,与 `origin/main` 同为 `d21d3d682`。
- **本轮分支 `claude/quota-reset-latch`**:3 个提交,见 §9.2。
- **5 个陈旧本地分支**:`backup/pre-backend-strip-branch`、
  `codex/full-identity-sync-cache-migration`、`codex/manual-reset-credits`、
  `codex/remove-provider-ads`(其 origin 侧已 gone)、`feat/frontend-redesign`。
  没人核实过它们是否还有独有价值 —— 删之前逐个跑
  `git log --oneline main..<branch>`,别直接 `-D`。
- **6 个 stash**:5 个是 `claude/llm-usage-monitoring-app-4a9554` 上的后端裁剪
  WIP(都标着 build red / over-reached),1 个是 main 上的文档备份。
- **老 tag**(如 `v3.8.3`)带着 CC Switch 上游血统的提交,不在 origin 的分支上。

也就是说:**没有代码因为"忘了推"而处于危险状态**,但仓库里确实堆着一批没人认领的
本地 ref。要清理的话按上面的顺序逐个核实,不要一把梭。

#### 2026-08-11 复查与清理(本节以此为准)

上面那份清单已按它自己给的方法逐条核实并执行完毕:

- **5 个陈旧本地分支全部 `main..<branch>` = 0**(落后 61 / 96 / 103 / 201 / 293),
  即内容一条不少地在 main 里,已用 `git branch -d` 删除(`-d` 而非 `-D`,让 git 再
  把关一次)。`backup/pre-backend-strip-branch` 另有同名 tag `backup/pre-backend-strip`
  留底。
- **`feat/provider-api-keys` 曾是唯一真正危险的东西** —— 8 个提交、49 个文件、
  +4839/−855,**只存在于本机,没有任何远端副本**。已推送并开 PR #24。
- **6 个 stash 一个没动。** 它们挂在 PR #17 那条已合并的分支上,每条自己的说明都写着
  失败原因(over-cut / over-reached / coupling deeper than scoped / blocked on /
  build red)—— 是失败的尝试,不是待合并的工作。丢弃是不可逆的,留着不花钱,交给用户决定。
- **`/Users/max/LLM-Usage-Bar`(无空格的那个目录)是一份死副本。** 8 月 2 号从同一个
  remote 克隆,`main` = `1cd4c824`,已是当前 main 的祖先(落后 67 个提交),独有提交
  **0 个**,无 stash、无本地分支、从未 fetch 过、工作区干净。里面没有任何要捞的东西 ——
  下次别再被它误导成"另一条线"。
- **worktree 只有主目录一个**,`.claude/worktrees/` 是空目录。
- reflog 里那个被 `reset HEAD~1` 丢掉的 `WIP: snapshot before splitting multi-key work`
  (`0e7edc108`)**没有丢东西** —— 它的树与 `feat/provider-api-keys` 顶端 diff 为空,
  内容原样拆进了那 3 个提交。

---

## 10. Provider 多 key 花费线(2026-08-11,PR #24,待合并)

`feat/provider-api-keys`,建在 `claude/quota-reset-latch` 之上,自己 3 个提交。

**为什么要做**:Provider 花费一直显示不出来,根因是 OpenRouter 的 preset 声明
`token_sources = [Proxy]`,而这个代码库里**根本不存在本地 proxy**,所以没有任何路径
能把用量归到它头上。改为直接读每把 key 自己的账单端点 —— 该端点是 key 维度的,这就是
"一个 Provider 一份凭据"必须改成"一列具名 key"的原因。

| 提交 | 层 |
| --- | --- |
| `64723e8b6` | 数据层:v25 加 `provider_key_usage_snapshots`;v26 换成 `provider_api_keys`,快照表与凭据日志重新挂到 `key_id` |
| `a1c833bc7` | 抓取/调度/暴露:preset 上的 `key_usage_path` 驱动 KeyUsage 端点(目前只有 `openrouter-api`);调度器每 15 分钟刷新每把 key;`UsageProviderView` 带上 `api_keys` 与 `key_usage_total` |
| `95fb1c1ae` | UI:Provider 卡片按名字列出每把 key,各自的状态点/连接测试/替换/删除/合计 |

**v26 在实库上安全的两条不变量**(改这块之前必读):

- **keychain slot 字符串原样搬运。** slot 是 OS keychain 里取密钥的查找键,改名会把每一
  份已存密钥变成孤儿。只有新建的 slot 才用 key 维度命名。
- **只有真正持有凭据的行才变成 key。** 每个 Provider 都有 credentials 行,其中大多是
  version 0 的空占位;照单全收会造出一批幽灵无名 key。

`binding_credentials.rs` 的 slot 存活检查也搬到了新表 —— 不搬的话,binding 清理会把一把
活着的 Provider key 的密钥当成孤儿删掉。

**bisect 注意**:`64723e8b6` 和 `a1c833bc7` 单独不可构建,三层是一起重写的。

**本机验证(2026-08-11 实跑,不是 CI 的结论)**:

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过,0 告警 |
| `cargo test` | 1106 passed / 0 failed(lib 1093 + 集成 13),2 ignored |
| `tsc --noEmit` | 通过 |
| `prettier --check` | 通过 |
| `vitest run` | 341 passed / 0 failed(52 个文件) |

**还欠的**:目视验证。app 从没用这个分支构建安装过。装之前记住 —— 生产库现在 v24、
装着的 3.16.5 也是 v24,**这个构建一上去库就迁到 v26,旧版 app 再也打不开,必须一步到位**。
最近备份 `db_backup_20260810_171554.db`。
