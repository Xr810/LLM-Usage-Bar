# LLM Usage Bar — 全局交接文档(合并版)

最后核实:2026-08-13(所有事实当天用 git / sqlite3 / grep 逐条验证过,不是抄旧文档)
最后更新:2026-08-13 —— ①分支与 stash 收敛、docs 重排,见 §0 与 §11.7;②§11.3 重写:
游标 N+1 与 tokio worker 数已由提交 `52bd4559` 修掉,该节改为「已完成 / 仍未做」两栏;
③§11.3 第三次更新:文件监听替代轮询(`bf46de69` + `e4ece596`)与 Codex 日期分区剪枝
(`d96543df` + 补漏 `96e08fd3`)已合入 main,两条移进「已完成」,该节只剩三条仍未做;
④ubuntu CI 红了两个剪枝测试(同秒 mtime 依赖),修于 `3b915f5f`,根因见 §2;
⑤窗口最小化检测事件化(`980aa620`,合并提交 `320d04c3`)合入 main,Swift 壳源码落
`feat/swift-native-shell` 并开 PR #27;⑥productIdentity pins 重锚(`0e7dd3cd`),
CI 前端红修复、main 上 CI 双绿;⑦任务 7 归因实测写入 §11.2,任务 6(QoS + 定时器
唤醒)落地,§11.3 五项全部完成

> **这是唯一的交接文档。** 它取代并吸收了以下分散文档,那些文件不要再单独更新:
>
> | 被取代的文档 | 原位置 | 状态 |
> | --- | --- | --- |
> | `docs/archive/task-state/current-task.md`(2026-07 版) | 主仓库 | 历史,PR #6 时代,已完成 |
> | `docs/archive/task-state/2026-07-12-dependabot-ci-failures-handoff.md` | 主仓库 | 作废 —— PR #4/#5 已被关闭重开为 #15/#18 |
> | 2026-08-07 checkpoint(`docs/archive/task-state/current-task.md` 更新版) | 分支 `claude/llm-usage-monitoring-app-4a9554` 提交 `67676ef9e` | 内容已并入本文 §2、§6、§7 |
> | `HANDOFF.md` 报告(一)(二) | worktree `usage-model-agent-classification-03acf1`(报告二未提交) | 内容已并入本文 §4、§5 |
>
> 交接方式:后续 agent 把新发现**追加到本文件**,不要另开新文档。

---

## 0. 开工前必做的三步(两次翻车都是因为跳过了这里)

1. **基线就是 `main`。** 2026-08-12 实测:本地 `main` = `origin/main` =
   `199fdbfc`,`SCHEMA_VERSION = 26`。之前"本地 main 落后 55 个提交"的问题
   **已经解决**,不要再从别的分支拉线。

   ```bash
   git fetch origin && git log --oneline -1 origin/main
   ```

   > 别照抄这里的哈希 —— 上面这条命令的输出才算数。远端**只有 `main` 一条活分支**;
   > PR #26(Swift 原生线第一阶段)的 base 分支 `Swift` 已从远端删除,其成果的
   > 找回方式见 §11.7 —— 那份**只存在于本机的 Swift 源码**现在钉在
   > `backup/swift-native-stash` 上(不再在 stash 里),动它之前必读 §11.7。
   >
> **2026-08-13 更新:本地还剩 3 个分支** —— `main`、`backup/swift-native-stash`、
> `perf/window-visibility-events`(300ms 窗口轮询的事件化改造,已于 2026-08-13
> 合入 main,分支保留,见 §11.3)。
> `codex/swift`(`50576652`)与 `pr-26-swift-merge`(`b841a53c`)已删除,`stash` 已清空,
> 理由与找回方式见 §11.7。

2. **动数据库/想本地跑 app 之前,先对版本。** 生产库 2026-08-11 实测仍是 **v24**:

   ```bash
   sqlite3 ~/.llm-usage-bar/llm-usage-bar.db "PRAGMA user_version;"
   ```

   分支的 `SCHEMA_VERSION`(`src-tauri/src/database/mod.rs:57`)必须 ≥ 库版本,
   否则 app 启动即崩:`authoritative database schema v24 is outside supported range …`。

   > **注意这里现在是错开的:代码 v26,库和已安装的 3.16.5 都还是 v24。** PR #24 合入后
   > main 就带着 v25、v26 两个迁移,但还没有人用它构建安装过。下一次构建安装会把生产库
   > 一路迁到 v26(自动做迁移前备份),**之后旧 app 再也打不开这个库,必须一步到位**。
   > 新迁移从 main 的 `SCHEMA_VERSION` 往上编,**下一个是 v27**。编号冲突正是 8 月两次
   > 翻车的直接死因,动手前先跑一遍上面那两条命令确认自己的基点。

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
| 主线 | `main` = `6050389ff` | 绿(ubuntu CI 双绿 + 本机 Rust 1106/0、前端 341/0 同 sha 实测),**代码 v26**(库仍 v24,见 §0 第 2 步),与 `origin/main` 同步(0 个未推) | 否 |
| Claude 额度重置时间 | **✅ 已合入 main(2026-08-11,PR #23,merge commit `eb3cd2e76`)** | 锁存已实现并验证;分支本地与远端均已删 | `/usage` PTY 探测仍待做 → §9.3 |
| Provider 多 key 花费 | **✅ 已合入 main(2026-08-11,PR #24,merge commit `c686ef884`)** | 3 个提交;**SCHEMA_VERSION 24 → 26**(两个迁移,各带 validator);本机全套 + ubuntu CI 双绿(见 §10);分支本地与远端均已删 | 目视验证未做 → P4 |
| 用量按模型/Agent 分类 | **✅ 已搬上 main(2026-08-07,提交 `e23894168`)** | Codex(max)在隔离 worktree 移植,Claude 逐 hunk 复核并独立重跑全套验证(Rust 1039/0、tsc、prettier、59+9 前端测试全绿) | 旧分支 `claude/usage-model-agent-classification-03acf1` 及其 worktree 已作废,可删(需 `-D`);目视验证仍欠 → P4 |
| 红绿灯燃烧速度投影 | **✅ 已搬上 main(2026-08-07,提交 `bb8514def`,迁移重编号 v23→v24)** | Codex(max)移植 + 签名脚本修复一并带上;Claude 复核(DDL 范围、预测行无机密、阈值为常量)并独立重验(Rust 1066/0、前端 192+9 全绿) | 旧分支 `claude/traffic-light-logic-redesign-3fbc8e` 及 worktree 可删;**注意:新代码 SCHEMA_VERSION=24,装上后旧 3.16.5 打不开升级后的库,须一步到位** |
| 性能优化线(§11.3) | `main` | 5 件全部完成:游标预载 + tokio worker 封顶(`52bd4559`)、Codex 日期分区剪枝(`d96543df` + 补漏 `96e08fd3`)、文件监听替代轮询(`bf46de69` + `e4ece596`)、300ms 窗口轮询事件化(`980aa620`)、同步线程 QoS + 定时器唤醒(任务 6);dispatch-timer leeway 留观察项 | 否 |
| 2026-08-07 checkpoint 文档 | `claude/llm-usage-monitoring-app-4a9554`(= main 的内容 + 1 个 docs 提交 `67676ef9e`) | 纯文档分支,内容已并入本文 | 可删分支和 worktree |
| 本合并任务 | `claude/consolidate-error-issues-4d8721` | 即本文件所在分支 | 合并进 main 让后续 agent 能看到 |
| 6 个 `codex/*` 旧线(7 月) | ~~`.worktrees/`~~ | **✅ 已清理(2026-08-07)**:6 个 worktree、6 个分支、3 个失效 bridge worktree 全部移除(删前核实 0 独有提交、工作区干净) | 否 |

> 更正一个旧文档里的错误结论:报告(二)说
> `claude/llm-usage-monitoring-app-4a9554`"不是 origin/main 的祖先,是另一条线"。
> 实测其父提交 `e337dd32e` **就是 main 的祖先**,该分支只是 main + 一个文档提交,
> 没有失散的代码。

### GitHub PR(2026-08-12 实查:#12–#26 已全部合并或关闭,0 个开着)

| PR | 内容 | 检查 | 行动 |
| --- | --- | --- | --- |
| ~~#26~~ | **Swift 原生菜单栏 app + UsageCore 库(迁移第一阶段)** | — | **✅ 已合(2026-08-11)** —— 但 base 是 `Swift` 分支而非 main,该分支已从远端删除,成果不在 main 上!找回方式与后续状态见 §11.7 |
| ~~#25~~ | frontend-deps 依赖组(5 项) | CI 绿 | **✅ 已合**,即当前 main 顶端 `199fdbfc` |
| ~~#24~~ | Provider 多 key + 每把 key 的花费 | 本机全套绿 + ubuntu CI 绿 | **✅ 已合(merge commit)** |
| ~~#23~~ | Claude 额度重置时刻锁存 | 绿 | **✅ 已合(merge commit)** |
| ~~#22~~ | frontend-deps 依赖组(40 项,全 minor/patch) | 合前在当前 main 上本机实测 341/0;合后 ubuntu CI 绿 | **✅ 已合(squash)** |
| ~~#21~~ | cargo-deps `base64` 0.23.0→0.23.1 | 合后本机 Rust 1106/0 + ubuntu CI 绿 | **✅ 已合(squash)** |

> #21/#22 合的时候 base 还停在 `d21d3d682`(落后 13 个提交),它们 PR 页上的绿是对旧
> 基底跑的 —— 那时多 key 的 UI 还不存在,而 #22 里有 16 个 Radix 包、React、
> `user-event`、`vite`,正好可能动到当天新增的 149 个前端测试。合之前在隔离 worktree
> 里把 #22 合到真实 main 上实测过(341/0,无冲突,`--frozen-lockfile` 通过),合之后
> 又在真实 main 上复验一次。**下次遇到 base 落后的 dependabot PR,先看它的绿是对哪个
> 基底跑的。** #25 没有这个问题,它 base 就是当前 main。
>
> 另:`node-pty` 已完全不在 lockfile 里,§P3 当初预测的"重建的 PR 不再牵涉 node-pty"
> 成立,那个供应链决策点已消失。

> #23 与 #24 **必须**用 merge commit 合,已照做。原因:#24 的前 5 个提交**就是** #23 的
> 那 5 个,squash 或 rebase 会重写哈希,同一份改动就会以两组提交出现在 main 的祖先里。
> 合完实测:main 的树与合并前的 `feat/provider-api-keys` **逐字节一致**,无重复提交 ——
> 也就是说本文 §10 记的那套本机验证结果原样适用于当前 main。

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
- **ubuntu CI 两个 Codex 剪枝测试红**(2026-08-13,修于 `3b915f5f`) —— 单一根因:测试
  场景靠「同一秒内真实文件操作的先后顺序」生效(scratch 文件 create+delete 抬目录
  mtime、append 抬文件 mtime),macOS 的 APFS mtime 是纳秒级所以本地全绿,ubuntu CI
  文件系统的 mtime 分辨率不足以区分同秒内的先后(实测两次操作的 mtime 效果完全不可见)。
  修法:测试改用 `filetime`(已在依赖树里,经 notify 引入)显式设时间戳,相邻值至少差
  2 秒,判定结果与分辨率无关;真实 append 保留在 resume 测试里继续记录「append 不改父
  目录 mtime」这一事实。生产代码不受影响(macOS/Windows 的 mtime 都是亚秒级)。
  → 教训:任何依赖「同秒内两次文件操作仍可区分先后」的测试,换文件系统就翻车 ——
  要造时间顺序就用显式时间戳,不要用真实时钟。

---

## 3. 待办问题清单(按优先级)

### ~~P0 — 把两个 PR 合上 main~~ ✅ 已完成(2026-08-11)

#23 与 #24 都已用 merge commit 合入,顺序正确(先 #23 后 #24)。合后实测:main
= `c686ef884`,`SCHEMA_VERSION = 26`,树与合并前的 `feat/provider-api-keys`
逐字节一致,无重复提交,ubuntu CI 绿。四条分支(两条已合、两条本地遗留)本地与
远端都已删除,本地只剩 `main`。**新迁移从 v27 起编。**

> 留作教训:这两条线是父子关系,#24 的前 5 个提交**就是** #23 的那 5 个。用 squash
> 或 rebase 合 #23 会重写它们的哈希,而 #24 仍带着原版 —— 同一份改动会以两组提交出现
> 在 main 的祖先里。下次遇到 stacked PR,先查父子关系再选合并方式。

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

新版已通过 `scripts/build_and_run.sh` 构建、签名、安装并正常运行;生产库已
迁移 v23→v24(迁移前备份 `db_backup_20260807_134610.db`),日志健康,
session 同步正常。**剩下的只是用户亲眼扫一遍**:三个 Tab(Providers /
Models / Agents)的渲染、新红绿灯与 pace 详情、暗色/亮色、窄窗口。
测试全绿但像素没人看过 —— 发现视觉问题记回本文件。

> 2026-08-11 补充:P0 的两个 PR 合完之后会有一个新构建,那一次可以把这里欠的一眼、
> §9.2 的重置时间显示、§10 的多 key 卡片一起看掉,不必分三次装。

### P5 — 低优先级 / 观察项

- **⚠️ 47 个本地 tag 全部未推,而且不要推(2026-08-11 实测)。** `git ls-remote --tags
  origin` 返回 0 条,GitHub 上既没有 tag 也没有 Release —— 看起来像遗漏,其实不能补。
  `.github/workflows/release.yml` 的触发条件是 `push: tags: ["v*"]`,其中 46 个 tag
  命中该模式,`git push --tags` 会**逐个**触发 macos-14 上的签名 release 构建;
  concurrency group 是 `release-${{ github.ref_name }}`(按 tag 名分组),所以它们
  **不会互相取消**,而且 workflow 带 `contents: write`,会建出 46 个 GitHub Release。
  另外 `v3.8.3`(21 个独有提交)和 `v3.1.2`(2 个)带的是 CC Switch 上游血统,推上去
  等于把上游发布史灌进本仓库(其余 45 个都在 `origin/main` 历史内,推了不新增对象)。
  `backup/pre-backend-strip` 不匹配 `v*`,是本地备份标记,本就该留在本地。
  **要发版就单推那一个 tag。** 这也是 §9.5 那个"未推提交总数"陷阱的另一半:那 23 个
  不在 origin 上的提交正是这两个 tag 带的,不是谁忘了推分支。
- ~~合并绿的 #12–#15~~ **✅ 已了结(2026-08-12 实查)**:#12/#13/#14/#15/#17 已合并,
  #16/#18 已关闭。上一版留的那条 `gh pr merge` 命令作废,不要再跑。
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
  且用户没有密码。`scripts/build_and_run.sh` 的修复(提交 `5ed753e08`,在
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

- `docs/design/2026-07-17-provider-only-monitoring-design.md` —— 产品方向定调
- `docs/design/2026-08-03-official-pricing-refresh-design.md` —— 官方价目刷新(已实现)
- `docs/design/2026-08-05-custom-pricing-usability-design.md` —— 自定义价格可用性(已实现,`317911b33` / `08dcb9dea`)
- `AGENTS.md` —— 包装器与 Kimi 委派规则
- `docs/testing/usage-dashboard-acceptance.md` —— 验收 runbook(PR #6 时代,流程仍可参考)
- 其余 `docs/archive/plans/*` 与 `docs/archive/2026-07-19-manual-reset-credits-design-qa.md` 为历史实施记录,只作考古用

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
  +4839/−855,**只存在于本机,没有任何远端副本**。已推送、开 PR #24 并当日合入 main。
  它是这次普查里唯一一件"真丢了就没了"的东西;其余全部是已并入 main 的陈旧 ref。
- **陈旧远端分支 `origin/codex/usage-dashboard-backend` 仍在**(0 独有提交,落后 247)。
  纯清理项:`git push origin --delete codex/usage-dashboard-backend`。
- **6 个 stash 一个没动。** 它们挂在 PR #17 那条已合并的分支上,每条自己的说明都写着
  失败原因(over-cut / over-reached / coupling deeper than scoped / blocked on /
  build red)—— 是失败的尝试,不是待合并的工作。丢弃是不可逆的,留着不花钱,交给用户决定。
  > 2026-08-12 更新:`git stash list` 现在**只剩 1 个 stash**(`stash@{0}`,codex/swift
  > 线的,**含唯一一份 Swift 源码,见 §11.7,勿丢**)。原来那 6 个已不在列表里,
  > 谁清的、何时清的没有记录 —— 按上一条的性质判断无实质损失,但这正是"清 stash
  > 不留痕"的例子,引以为戒。
  >
  > 2026-08-13 更新:**stash 列表现在是空的。** 最后那个 `stash@{0}` 已 drop —— 它和
  > `backup/swift-native-stash` 是同一个提交 `296346b8`,分支还在,源码一字未丢,
  > 见 §11.7。这次删除有记录,不重蹈上面那笔。
- **`/Users/max/LLM-Usage-Bar`(无空格的那个目录)是一份死副本。** 8 月 2 号从同一个
  remote 克隆,`main` = `1cd4c824`,已是当前 main 的祖先(落后 67 个提交),独有提交
  **0 个**,无 stash、无本地分支、从未 fetch 过、工作区干净。里面没有任何要捞的东西 ——
  下次别再被它误导成"另一条线"。
- **worktree 只有主目录一个**,`.claude/worktrees/` 是空目录。
- reflog 里那个被 `reset HEAD~1` 丢掉的 `WIP: snapshot before splitting multi-key work`
  (`0e7edc108`)**没有丢东西** —— 它的树与 `feat/provider-api-keys` 顶端 diff 为空,
  内容原样拆进了那 3 个提交。

---

## 10. Provider 多 key 花费线(2026-08-11,PR #24,✅ 已合入 main)

原 `feat/provider-api-keys`(建在 `claude/quota-reset-latch` 之上,自己 3 个提交),
已用 merge commit `c686ef884` 合入 main,分支本地与远端均已删除。

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

合并后 ubuntu CI 同样全绿(Backend Checks 12m56s、Frontend Checks 2m37s)。这一条要紧:
按 §2 的教训,本机是 macOS 而 CI 是 ubuntu,`cfg(target_os)` 形状的缺陷本地编译不到。
另外合并后实测 main 的树与该分支**逐字节一致**,所以上表结果原样适用于当前 main。

**还欠的**:目视验证。**没有任何人用 v26 的代码构建安装过 app。** 生产库现在 v24、
装着的 3.16.5 也是 v24,**下一个构建一上去库就迁到 v26,旧版 app 再也打不开,必须一步
到位**。最近备份 `db_backup_20260810_171554.db`。要看的:每把 key 一行的 Provider 卡片、
展开后的日/月数字与剩余预算、只有一把 key 时不显示合计、被替换凭据的数字有标注、
以及 §9.2 的重置时间行 —— 连同 P4 欠的那一眼一次看完。

---

## 11. 功耗根因与技术路线评估(2026-08-12,全部实测)

### 11.1 结论先行

**菜单栏 app 的耗电与 UI 技术栈无关,全部来自后端自己的采集循环。** 换 UI 框架
(原生也好、Slint 也好)对这条线的收益接近于零 —— 这一条推翻了动手前的直觉判断,
动第 3 步之前先看完本节。

### 11.2 实测数据(生产版本 v3.16.5,PID 776)

| 指标 | 数值 |
| --- | --- |
| 累计 CPU / 运行时长 | 22:03 / 22h45m → **平均 1.62%** |
| 瞬时稳态(20s 窗口 ×3) | 3.0–3.2% |
| RSS | 46 MB |
| 线程数 | 18(含 **10 个 tokio worker**) |

> 采样期间本机在跑 Claude Code,会实时产生日志供摄取,瞬时 3.0% 含合理工作量。
> **22 小时均值 1.62% 是可信数字。**

按线程拆 CPU 时间:

| 线程组 | 累计 CPU |
| --- | --- |
| WebView 相关(CVDisplayLink ×2、WebCore Scrolling、JSC scavenger 等 6 个) | **0:00.00** |
| tokio worker + 临时派生线程(8 个) | **9:53** |

**WebView 线程是字面意义上的零。** 但要注意两个测量边界(2026-08-12 复核时指出):

- **这张表只覆盖 app 进程内的线程。** macOS 上 WKWebView 是进程外架构,渲染/GPU/网络
  跑在独立的 `com.apple.WebKit.{WebContent,GPU,Networking}` XPC 进程里,它们的 CPU 和
  内存都不在上表、也不在 46 MB RSS 里。闲置且窗口隐藏时大概率同样接近零,但"UI 层
  零开销"这个结论目前证据链不完整,要下定论需按进程组重测。
- **归因缺口:** tokio 组只解释了 9:53,总量 22:03 里**还有约 12 分钟(~55%)没有归属**
  (最可能在主线程:tray 刷新、事件循环、下述 300ms 轮询)。所以"轮询浪费 ≈ 1,382 秒/天"
  是把全部均值 CPU 都记在了轮询头上,**修完轮询预期只能收回一部分,动手前先把剩余
  12 分钟归因清楚**(用 Instruments Time Profiler 对主线程采样即可,它对 Rust 二进制
  完全可用)。

  #### 2026-08-13 实测归因(任务 7 产出,全部实测)

  原测量对象 PID 776 已随 app 重启消失,22:03 的历史归因无法重测。对**当前运行的同
  一版本实例**(v3.16.5,PID 811,采样时已运行 76 分钟、累计 CPU 2:06,平均 ~2.8%)做
  了两组实测:

  - **线程级累计 CPU(`ps -M`)**:主线程 ~50.7s(**~40%**);其余 ~75.9s(~60%)分布在
    ~8 个活跃线程(单个 5–17.4s,分布均匀,与 10 个 tokio worker + 驱动线程的结构
    吻合)。即「未归因」的那部分在当前实例里主要落在**主线程之外**的线程组。
  - **60 秒 Time Profiler 采样(`sample`,1ms 间隔,~6 万样本)**:
    - 主线程 48,847 样本中 **47,906(98%)阻塞在 runloop 的 mach_msg 等待**,活动样本
      集中在 WebKit IPC 消息分发(~450)与事件处理 —— 主线程 CPU 是**事件驱动的突发**,
      不是常驻轮询;
    - 忙碌的 tokio worker:~2% 样本在 `pread` 链(SQLite 游标读取,60 秒同步扫描),
      11% 在 `kevent`(IO 驱动线程被定时器反复唤醒),其余在 `pthread_cond_wait`(park);
    - **修正 §11.2 的原假设**:v3.16.5 里 300ms 窗口轮询是 `tauri::async_runtime::spawn`
      到 **tokio worker** 上的,不在主线程 —— 「12 分钟未归因最可能在主线程」应改为
      「更可能在 tokio 组与驱动线程(定时器唤醒与同步扫描)」。与 §11.5 的结论一致:
      能耗几乎全部来自架构(唤醒频率、每次扫描量),不在主线程。
  - **测量边界(新增发现)**:发行版二进制带 `strip = "symbols"`,采样里全部 Rust 帧
    显示为 `???`(只有地址)。线程级归因可用,**函数级归因不行** —— §11.8 说的
    「Instruments 对 Rust 二进制完全可用」在进程级/线程级成立,函数级需符号。
    下次构建安装若想重做函数级复测,保留 debug symbols 或产 dSYM。

**结论**:剩余优化的目标确认在 **tokio 侧**(定时器唤醒方式 + 同步线程 QoS),
不在主线程 —— 这两条已随任务 6 落地(worker 线程 Utility QoS、5 秒兜底加
`MissedTickBehavior::Delay`,见 §11.3)。v3.16.5 的 300ms 轮询(在 tokio 上)与
60 秒全量扫描在 main 上已分别被事件化与文件监听 + 分区剪枝消除;main 代码里剩的
固定唤醒源是 5 秒最小化兜底定时器与 15 分钟 `mark_all_dirty`。

### 11.3 根因(2026-08-13 更新:五项全部完成,见本节末尾)

`lib.rs` 的 60 秒同步定时器(当时的 `SESSION_SYNC_INTERVAL_SECS = 60`,该常量已随
`e4ece596` 删除)要遍历:

```
~/.claude/projects        71 个 jsonl,111 MB
~/.codex/sessions       1294 个 jsonl,2.2 GB
```

同步**本身是增量的**(存 (mtime, line_offset) 游标,内容未变就跳过解析),
**但游标当时是逐文件查库的** —— 每个文件一次独立 SQLite 查询,而且查询发生在
"mtime 未变则跳过"**之前**,所以跳过也省不掉查询。

于是每 60 秒:~1,365 次 stat + ~1,365 次 SQLite 查询,折合**持续每秒 ~45 次磁盘
操作**(1,365×2/60;更早一版写 23 是只算了一类)。采样栈里 `pread` 高频出现,吻合。

> **2026-08-13 修正:上面这段的 SQLite 那一半已经不成立了。** 提交 `52bd4559`
> 把四个 source 的游标全部改成"每趟同步预载一次"。别再照抄"~1,365 次 SQLite
> 查询"这个数去论证任何事 —— 现在是每个 source 一次。
> 文件系统那一半(1,294 次 `File::open`)当时每 60 秒发生一次 —— 2026-08-13
> 已由事件驱动同步(`e4ece596`)与日期分区剪枝(`d96543df`)消除,见下。

#### 已完成

**游标预载 + tokio worker 封顶(提交 `52bd4559`,2026-08-13):**

- **游标批量预载**:新增 `SyncCursorMap`(`services/session_usage.rs:30`)与
  `load_sync_cursors()`(同文件 `:503`)。Claude 用不可变预载 map(每个文件的
  cursor key 唯一,趟内不可能失效);Codex 用可变 map + 第二张 `sync_cursor_details`
  存完整记录供 resource-identity 判断,legacy 路径游标被提升时同步从 map 里删掉
  (`services/session_usage_codex.rs:266-280`);gemini / opencode 一并预载。
  `get_sync_state` 已降级为 test-only。
- **tokio worker 数封顶**:`lib.rs:454-459` 显式建 2 worker 的 multi-thread runtime,
  再 `tauri::async_runtime::set`,不再用 Tauri 的默认值(原来是 10 个)。
- 该提交自述 `--lib 1094 passed / 0 failed`;它是从一个 `/var/folders` 下未提交的
  Codex 委派 worktree 里抢救回来的,除 rebase 到当时的 main 外未作修改。

**Codex 日期分区剪枝(提交 `d96543df`,补漏 `96e08fd3`,2026-08-13):**

- `d96543df`:`sessions/YYYY/MM/DD` 分区,日期早于 fresh 窗口且「每个 .jsonl 都有
  有效游标、文件 mtime 不晚于游标、目录 mtime 不晚于分区内最大游标」三条全满足才整
  分区跳过,判断全程只 read_dir / stat,**不开任何会话文件**(游标预载后按
  `resource_path` 建索引,见上)。模拟 1294 文件树实测:一趟同步 `File::open`
  **1294 次 → 94 次**(窗口关到极端值时必须与改前逐字段一致,有测试断言)。
  新增 `SessionSyncResult.files_pruned` 字段。
- `96e08fd3` 补上一个真漏洞:`codex resume` 会往老分区的原 rollout 文件继续 append,
  而 **append 不改父目录 mtime** —— 分区一旦被剪,续写部分就永久不再同步。实测 1296
  个会话文件里 4 个最后一条记录晚于分区日期 2 天以上,最长 +63 天、续写部分累计
  **2085 万 token** 会被静默丢弃;现在只剪「逐文件扫描也会全部跳过」的分区。

**文件监听替代轮询(提交 `bf46de69` + `e4ece596`,2026-08-13):**

- `bf46de69`:`src-tauri/Cargo.toml:29` 加 `notify = "8.2"` 依赖。
- `e4ece596`:用文件系统事件(FSEvents / ReadDirectoryChangesW / inotify)驱动同步,
  替代 60 秒全量轮询。新增 `src-tauri/src/usage/watcher.rs`(483 行),把原本
  全仓库零调用方的 `usage/watcher_state.rs`(445 行 dirty-generation 调度骨架:
  按源去抖、逐源失败退避 60s–86400s、防重入)接上线 —— `lib.rs:1164-1169` 建
  `WatcherSchedule` + `start_usage_watcher`;15 分钟 `mark_all_dirty` 兜底
  (`SLOW_FALLBACK_INTERVAL_SECS`,`watcher.rs:27`);退出路径调
  `stop_usage_watcher()`(`lib.rs:1540`)。**旧结论
  「`watcher_state.rs` 零调用方、`Cargo.toml` 没有 `notify`」已经作废,
  不要再照做"接线"。**

**300ms 窗口轮询事件化(提交 `980aa620`,2026-08-13 经合并提交 `320d04c3` 进 main):**

- 主窗口最小化检测从 300ms 常驻轮询改为事件驱动:`Focused(false)` 挂在 builder 级
  `on_window_event`(轻量模式会销毁重建主窗口,builder 级对每次新建都生效);
  判定逻辑抽成 `handle_minimized_main_window`(`lib.rs:425`),事件回调与
  5 秒兜底定时器共用(兜底覆盖「窗口非 key 时被最小化」这类不产生 Focused 变化的
  场景,`lib.rs:456`)。`STOP_MAIN_WINDOW_VISIBILITY_MONITOR` 原子量与
  `stop_main_window_visibility_monitor` 删除。合入时补了纯函数
  `classify_minimize_check_event`(`lib.rs:418`,判定哪些 WindowEvent 触发最小化
  检查)及其单测 —— 实现本体在 `#[cfg(all(target_os = "macos", not(test)))]`
  里测试编译不到,判定条件必须抽成不带 cfg 的纯函数。

**同步线程 QoS + 定时器唤醒(2026-08-13,任务 6):**

- **QoS**:runtime builder 加 `on_thread_start`,所有 worker/blocking 线程在 macOS 上
  设 Utility QoS(`pthread_set_qos_class_self_np`,libc)。用量同步、事件驱动循环、
  5 秒最小化兜底全在这个 runtime 上 —— 系统调度会排到 E-core 并配合 App Nap;
  主线程(UI)不受影响。这段是 cfg 门控,ubuntu CI 编译不到,本地 clippy/test/fmt
  全绿是唯一的门。
- **定时器唤醒**:60 秒全量轮询此前已由文件监听替代(事件驱动 + 15 秒超时等待,
  等待本身几乎不耗电);剩下的 5 秒最小化兜底定时器加了 `MissedTickBehavior::Delay`
  (睡醒/挂起恢复后不追补错过的 tick,唤醒节奏对 timer coalescing 更友好)。
  **未做**:真正的 dispatch-timer leeway / `NSBackgroundActivityScheduler` —— 需要
  objc2-foundation 或 dispatch FFI,而 §11.2 的 2026-08-13 实测归因显示剩余固定唤醒
  源只有 5 秒兜底(12 次/分,每次只读一次 Dock 可见性)与 15 分钟 mark_all_dirty,
  收益远小于成本。FSEvents 的 latency 在 notify 8.2 里硬编码为 0 且不暴露配置口
  (`watcher.rs` 顶部已记录),节流语义由脏代数 + 60 秒最小同步间隔承担。
  留作观察项。

**这一项与 UI 选型正交,选哪条路线都必须修** —— 方案 B 是绞杀者模式、Rust 采集层
保留,所以这里的改动在 SwiftUI 迁移之后依然有效(唯一例外是窗口最小化监控里的
5 秒兜底定时器,它最终会随外壳一起被 SwiftUI 取代)。

### 11.4 Rust vs Swift 核心层基准测试

真实日志(37 MB / 8,921 行 / 3,916 条 usage),两侧实现等价解析,**输出的 token
总数逐字节一致**。Rust 1.95.0 `--release`+LTO;Apple Swift 6.3.3 `-O -wmo`。

| | Rust (serde_json) | Swift (JSONDecoder) |
| --- | --- | --- |
| 耗时(3 次) | **17.1 / 18.0 / 21.8 ms** | **66.3 / 68.1 / 86.8 ms** |
| 该任务峰值 RSS | **3.5 MB** | 44.7 MB(见下) |
| 空程序基线 RSS | **1.4 MB** | 5.5 MB |

**CPU:Rust 快约 3.5–4 倍。**

**内存那个 44.7 MB 不能直接比** —— Swift 版一次性把 37 MB 文件读进内存,那是实现
方式不是语言开销。扣掉后工作集约 7.7 MB,对 3.5 MB 约 2 倍。另写的流式 Swift 版本
因 `Data.subdata` 重复拷贝反而劣化到 355 ms,**该数字已废弃,别引用**。

可信的内存结论只有两条:**基线 Swift 高约 4 MB,工作集约 2 倍。**

### 11.5 换算到真实负载(估计,非实测)

同步是增量的,稳态每分钟只解析新增几行:

| 场景 | Rust | Swift | 差值 |
| --- | --- | --- | --- |
| 首次全量导入 2.3 GB | ~1.1 s | ~4.2 s | 3 秒,一次性 |
| 稳态每分钟增量 | ~0.05 ms | ~0.2 ms | 0.15 ms |
| **日均 CPU 差异** | — | — | **< 1 秒/天** |

对照:**11.3 的轮询浪费约 1,382 秒 CPU/天。相差三个数量级。**

**所以:对本应用负载,Rust 与 Swift 的运行时能耗/内存差异可忽略。** 能耗几乎完全由
架构(唤醒频率、每次扫描量)决定,不由核心语言决定。

边界:本基准只覆盖 JSON 解析(核心最重的计算),未覆盖 ARC 在其他路径的开销、
SQLite 层(两侧同一个 C 库)、HTTP 层(I/O 等待为主)。后两者判断不足以翻盘,
但这是判断不是测量。

### 11.6 代码规模(供选型参考)

| | 规模 |
| --- | --- |
| Rust 总量 | 94,476 行 / 132 文件 |
| 与 Tauri 耦合 | 20,214 行 / 35 文件(21%) |
| 平台无关纯核心 | ~74,000 行(79%) |
| 前端 | 24,515 行 TS/TSX,83 组件 |

### 11.7 Swift 原生迁移线的真实状态(2026-08-12 重写 —— 上一版本节是错的)

> **上一版说 `native/` "只有空的 `.build` 骨架,源码为零,不要当起点" —— 错。**
> 错因:只 `ls` 了工作区。源码确实不在工作区,但它在 stash 和已合并的 PR 里。
> 以下逐条实测(git / gh 直查):

**这条线实际已经开工,且方向正确:**

- **PR #26 已合并**(2026-08-11,"Add Swift native macOS menu-bar app and UsageCore
  library (initial migration stage)"):10 文件 / +492 行 —— `native/Package.swift`、
  `UsageCore`(快照模型、statusline 源、原子 JSON 存储)、`MenuBarExtra` app 骨架、
  测试,以及 **`docs/native-swift-migration.md` —— 绞杀者模式的五阶段迁移路线**
  (保留 Rust 数据层与 SQLite schema,Swift 只读展示,最后才决定 Rust 核心去留)。
- **但 #26 的 base 是 `Swift` 分支,不是 main,且该分支已从远端删除** —— 远端现在
  只剩 `main`,这 492 行**不在 main 的历史里**。找回锚点:本地备份分支
  `pr-26-swift-merge`(= 被删分支顶端 `b841a53c`),或 `refs/pull/26/head`。
- **比 PR #26 先进得多的一版源码,现在在 `backup/swift-native-stash^3`**
  (原 `stash@{0}` 的 untracked 部分,stash 已于 2026-08-13 清空,分支同一个 SHA
  `296346b8`,内容一字未变)。2026-08-13 实测:该树共 **48 个文件、其中 30 个 `.swift`**,
  **不含任何 `.build` 产物**。取出方式:

  ```bash
  git ls-tree -r --name-only backup/swift-native-stash^3
  git checkout backup/swift-native-stash^3 -- native docs src src-tauri
  ```

  > **`native/` 和 `docs/` 不是全部。** 2026-08-13 实测,该树里还有 5 个桥接侧文件,
  > 只 checkout `native docs` 会**静默漏掉**它们:
  > `src-tauri/src/native_bridge.rs`、`src/types/nativeBridge.ts`,以及三份契约测试
  > `dashboardContract.test.ts` / `nativeSettingsContract.test.ts` /
  > `trayUsageContract.test.ts`。Swift 侧靠这几个文件跟 Rust 对接。

  内容为 UI 文件(`MainWindowView`、`ProviderActivityHeatmap`、
  `BreakdownDashboardViews`、`NativeSettingsView`、`NativeDesignSystem`、`L10n` 等)
  + 扩展的 `UsageCore`(`DashboardRepository`、`NativeBridgeClient`、
  `TrayUsageSnapshotV1`、`DashboardModelsV1`)+ Xcode 工程(Preview/Production 两个
  scheme)+ `docs/native-feature-matrix.md`。本机那个跑了 20+ 小时的
  "LLM Usage Bar Native Preview" 进程(**11 MB RSS / 0.0% CPU**,对照 Tauri 版
  32 MB / 1.0%)就是它构建的 —— 这也是目前唯一一组原生 vs Tauri 的同机实测对照。
- **抢救状态(2026-08-12 做的钉住,2026-08-13 收敛)**:唯一需要保留的是
  `backup/swift-native-stash`(`296346b8`)。已删除的两个及理由,2026-08-13 逐条实测:

  | 已删 | SHA | 实测理由 |
  | --- | --- | --- |
  | `codex/swift` | `50576652` | 6,834 个文件**全是 `.build` 产物**;仅有的 2 个 `.swift` 是 SwiftPM 生成的 `runner.swift` 和 `resource_bundle_accessor.swift`。人写的源码零行。 |
  | `pr-26-swift-merge` | `b841a53c` | 它的 7 个 `.swift` **全部**被 `backup/swift-native-stash^3` 覆盖(5 个 blob 完全相同,`Package.swift` 与 `LLMUsageBarNativeApp.swift` 的 stash 版更新);且 `refs/pull/26/head`(`bec36ef7`)仍在 GitHub 上。 |
  | `stash@{0}` | `296346b8` | 与 `backup/swift-native-stash` **是同一个提交**,drop 不丢任何字节。 |

  两条删除都能从 reflog 或上表的 SHA 找回。**`backup/swift-native-stash` 的源码内容
  已随 `feat/swift-native-shell`(PR #27,2026-08-13)上远端,本机分支仍原样保留。**

**待办(按顺序):**

1. ~~把 `backup/swift-native-stash^3` 的源码落成正式分支推上远端~~ —— **✅ 已完成
   (2026-08-13)**:分支 `feat/swift-native-shell`(48 文件、30 个 `.swift`、
   0 个 `.build`,与 stash 树逐字节一致;两份文档落在 `docs/design/`;三份前端
   契约测试本地 3/3),见 **PR #27**。源码现在 GitHub 上有副本。
2. ~~删除 `codex/swift`~~ —— 2026-08-13 已删,见上表。
3. 两份 Swift 相关文档(`docs/native-swift-migration.md`、`docs/native-feature-matrix.md`
   —— 2026-08-13 实测就是两份,不是之前写的三份)目前也只活在
   `backup/swift-native-stash^3` / PR #26 里,随第 1 步一起落地。落地时注意本仓库
   `docs/` 已在 2026-08-13 重排(见 `docs/README.md`):这两份属于"活文档",
   应放进 `docs/design/`,不要落在 `docs/` 根上。

### 11.8 选型结论(2026-08-12 评审复核后)

评审输入材料见 **`docs/design/tech-route-review-2026-08-12.md`**;复核意见要点:

- 其主结论(§5.5,"能耗由架构决定,不由核心语言决定")**成立**,数据可信;
- 但评审文档 §6 有两行夸大了 Swift 优势:Instruments 对 Rust 二进制同样可用
  (进程级、基于符号,保留 debug symbols 即可),不构成换语言的理由;
  FSEvents/Keychain 在 Rust 侧有 `notify` / `keyring` 等成熟封装,不需要裸 objc2;
- 流式 Swift 解析 355ms 的劣化是 `Data.subdata` 强制拷贝所致(切片本是零拷贝视图),
  是实现问题不是语言问题,别拿它论证"Swift 内存必然差"。

**结论:方案 B —— Rust 核心 + SwiftUI 壳。** 这正是 PR #26 已经开工的绞杀者路线,
**继续它,不要另起炉灶**。已定背景条件不变:macOS 优先、Windows 押后不封死、
改写工作量不作为约束。阶段 1 的"Rust 导出 JSON 快照、Swift 只读"跑通后,长期可
升级为 UniFFI 直接绑定(自动生成 Swift 绑定,省掉快照文件中介)。

**优先级不变:先修 11.3(收益比整个选型问题大三个数量级),UI 迁移按绞杀者节奏
并行,互不阻塞。** 动 11.3 之前先把 §11.2 指出的 ~12 分钟未归因 CPU 查清楚。
