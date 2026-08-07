# LLM Usage Bar — 全局交接文档(合并版)

最后核实:2026-08-07(所有事实当天用 git / gh / sqlite3 逐条验证过,不是抄旧文档)
最后更新:2026-08-07 下午 —— 小项修复已执行,见 §3 各条目内的 ✅ 标记

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

1. **基线就是 `main`。** 2026-08-07 起本地 `main` = `origin/main` = `f3aadadec`
   (“Refocus on quota and spend… (#17)”),`SCHEMA_VERSION = 23`。
   之前"本地 main 落后 55 个提交"的问题**已经解决**,不要再从别的分支拉线。

   ```bash
   git fetch origin && git log --oneline -1 origin/main
   ```

2. **动数据库/想本地跑 app 之前,先对版本。** 生产库当前 v23:

   ```bash
   sqlite3 ~/.llm-usage-bar/llm-usage-bar.db "PRAGMA user_version;"
   ```

   分支的 `SCHEMA_VERSION`(`src-tauri/src/database/mod.rs:56`)必须 ≥ 库版本,
   否则 app 启动即崩:`authoritative database schema v23 is outside supported range …`。
   新迁移一律从 main 的版本号往上编(当前下一个是 v24)。

3. **读完本文件再动手。** 8 月的两条 feature 线全栽在同一个坑
   (基点过时 → 迁移编号错 → 装上就崩),第二次翻车时答案已经写在交接文档里了。

---

## 1. 现状总览(2026-08-07 逐条核实)

### 生产环境:健康

- 已安装 app:`/Applications/LLM Usage Bar.app`(3.16.5,能正常打开 v23 库)。
- 生产库:`~/.llm-usage-bar/llm-usage-bar.db`,v23,65 MB,45k+ 条 `usage_events`,
  `PRAGMA integrity_check` = ok,数据正常增长。**数据库从未损坏过** ——
  8 月的两次"崩溃"都是版本上限检查拒绝打开旧代码,不是写坏。
- 备份:`~/.llm-usage-bar/backups/`(v19→v21→v22→v23 各阶段都有)。
- 日志:`~/.llm-usage-bar/logs/llm-usage-bar.log`;崩溃日志 `~/.llm-usage-bar/crash.log`。

### 各条线的状态

| 线 | 分支 / 位置 | 状态 | 需要行动? |
| --- | --- | --- | --- |
| 主线 | `main` = `f3aadadec` | 绿,v23,与 origin 同步 | 否 |
| 用量按模型/Agent 分类 | `claude/usage-model-agent-classification-03acf1`,worktree `.claude/worktrees/usage-model-agent-classification-03acf1` | 功能完成、测试全绿、**已本地提交**(`80d791725`)、未推送;**基点过时,跑不起来** | **是 → §4** |
| 红绿灯燃烧速度投影 | `claude/traffic-light-logic-redesign-3fbc8e`,worktree `.claude/worktrees/traffic-light-logic-redesign-3fbc8e` | 功能完成、已提交已推送(`45898d3ba`);**基点过时,装上即崩,已回滚**;签名脚本修复已补提交并推送(`5ed753e08`) | **是 → §5** |
| 2026-08-07 checkpoint 文档 | `claude/llm-usage-monitoring-app-4a9554`(= main 的内容 + 1 个 docs 提交 `67676ef9e`) | 纯文档分支,内容已并入本文 | 可删分支和 worktree |
| 本合并任务 | `claude/consolidate-error-issues-4d8721` | 即本文件所在分支 | 合并进 main 让后续 agent 能看到 |
| 6 个 `codex/*` 旧线(7 月) | ~~`.worktrees/`~~ | **✅ 已清理(2026-08-07)**:6 个 worktree、6 个分支、3 个失效 bridge worktree 全部移除(删前核实 0 独有提交、工作区干净) | 否 |

> 更正一个旧文档里的错误结论:报告(二)说
> `claude/llm-usage-monitoring-app-4a9554`"不是 origin/main 的祖先,是另一条线"。
> 实测其父提交 `e337dd32e` **就是 main 的祖先**,该分支只是 main + 一个文档提交,
> 没有失散的代码。

### GitHub PR(2026-08-07 实查)

| PR | 内容 | 检查 | 行动 |
| --- | --- | --- | --- |
| #18 | frontend-deps 依赖组(接替被自动关闭的 #16) | **红** | 见 §3 P3,需要用户拍板 |
| #15 | cargo-deps 依赖组 | 绿 | 可合并 |
| #14 | actions/labeler 6→7 | 绿 | 可合并 |
| #13 | actions/stale 10→11 | 绿 | 可合并 |
| #12 | actions/setup-node 6→7 | 绿 | 可合并 |

7 月文档里的 PR #4/#5 巨型 Dependabot PR 已不存在,那份 handoff 的修复指引不再适用
(但它建议的"Dependabot 分组限制 minor/patch"策略仍未落实,见 §3 P5)。

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

### P1 — 把「分类」线搬上 main(§4 有完整步骤)

功能做完、2428 个 Rust 测试 + 65 个前端测试全绿,但基于 55 个提交前的旧基点,
在 v23 面前跑不起来。改动**未推送**,只存在于本地提交 `80d791725`。

### P2 — 把「红绿灯」线搬上 main(§5 有完整步骤)

同样的基点问题。~~抢救签名脚本修复~~ **✅ 已完成(2026-08-07)**:
`script/build_and_run.sh` 的 +120 行签名修复已提交为 `5ed753e08` 并推送到
`origin/claude/traffic-light-logic-redesign-3fbc8e`,丢失风险解除。

### ~~P3 — PR #18:`node-pty` 构建脚本放行~~ ✅ 已解决(2026-08-07,用户拍板)

用户选择**关闭 #18**而非放行 `node-pty` 的 postinstall。分组策略已限为
minor/patch(见 P5),Dependabot 下个周期会重建一个不含 major 的小 PR,
大概率不再牵涉 `node-pty`。若重建的 PR 仍要求放行某个传递依赖的构建脚本,
那依旧是供应链决策 —— **问用户,不要默默加允许列表**。

### P4 — 目视验证欠账

「分类」线的三个 Tab(Providers / Models / Agents)**从未被人眼确认过**
(五次尝试都被 schema 崩溃或 harness 杀进程挡住)。搬上 main 之后必须补:
实际渲染、暗色/亮色、窄窗口。正确姿势:
先 `osascript -e 'quit app "LLM Usage Bar"'`,再在 worktree 前台跑 `pnpm dev`。

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
