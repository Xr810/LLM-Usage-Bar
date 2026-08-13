# 本地路由与多 Provider 故障转移设计草案

> **状态:草案(未决策)**。本文是评审输入,不是已批准的规格。
> 按 `docs/README.md` 的约定:批准后成为 live 规格;被否决或取代后移入 `archive/`。
>
> **日期:** 2026-08-13
> **提出人:** 用户(2026-08-13 讨论整理)
>
> **2026-08-13 补充:** §1 R4 远端共享拓扑已由用户拍板(本地路由主体仍待评审)。
>
> **相关文档:**
>
> - [`2026-07-16-agent-provider-switching-design.md`](2026-07-16-agent-provider-switching-design.md) —— 被取代的切换设计(未落地)
> - [`2026-07-17-provider-only-monitoring-design.md`](2026-07-17-provider-only-monitoring-design.md) —— 当前批准的只读监控定位(本草案将部分撤销)
> - [`tech-route-review-2026-08-12.md`](tech-route-review-2026-08-12.md) —— 核心语言与 UI 选型评审(低功耗约束)
> - `HANDOFF.md` §0/§1 —— 仓库现状;§"v21 删除" —— 代理功能退役记录
> - `docs/user-manual/zh/4-proxy/*` 与 `docs/guides/codex-official-auth-preservation-guide-zh.md` —— 被删除功能的完整文档(历史)

---

## 0. TL;DR

用户提出三件事,本草案逐条回应:

| 需求 | 内容 | 本草案的结论 |
| --- | --- | --- |
| **R1** | 一个模型可以配置多个 provider,某个 provider 不可用时自动路由到下一个可用 provider | 可行。这是旧 CC Switch 代理"故障转移队列 + 熔断器"的功能子集,代码已在 v21 删除,文档仍完整。本草案恢复并以**配额感知**增强(旧实现没有)。 |
| **R2** | 用"多个配置文件 + 指针"替代 CC Switch 的复写式切换,避免写坏配置 | **主线:CLI 配置只写一次指针(指向本地路由),切换全部发生在路由内部**。用户启动命令固定(终端固定命令 / GUI 图标 / IDE 均不携带参数),配置文件是唯一被所有启动方式读取的地方,因此"写一次指针"是唯一全覆盖方案;"启动时带参数"的方案只覆盖终端,降级为可选增强(见 D9)。 |
| **R3** | Codex 先官方登录(保留插件/远程),本地路由把实际 provider 换掉 | 可行,旧实现已做过:第三方信息只写 `config.toml`,`auth.json` 保留官方登录。需在本仓库架构下实测验证(见 Q1)。 |
| **R4** | 合买订阅 + VPS 反代的远端共享拓扑 | **已拍板(2026-08-13)**:VPS v1 只做纯反代(Caddy 注入共享凭证),不当 router、不跑 LiteLLM;本地 router 把它当普通逻辑 provider。计量放客户端(朋友各自跑本 app 监控同一账号);仅当出现滥用/分账需求时才在 VPS 上升级 LiteLLM **网关**(虚拟 key + 限额),仍不开它的路由。见 §1 R4 与 §7。 |

**关键事实:这三件事在本仓库的 CC Switch 时代全部实现过。** schema v21(已合入 main)删除了九张退役表,其中四张直接属于代理/故障转移:`proxy_config`、`proxy_live_backup`、`provider_health`、`stream_check_logs`(`schema.rs:1404-1418`)。因此这不是新发明,而是**恢复已删除功能 + 针对旧实现的失败模式重新架构 + 接入本 app 独有的配额数据**。

**不需要 LiteLLM。** 旧实现证明这套功能在本代码库技术栈(Rust/TS)内完整可行;LiteLLM 是 Python 常驻服务,与 tech-route-review 确立的低功耗前提冲突(详见 §7 Non-goals)。

---

## 1. 需求

### R1:一模型多 provider,自动故障转移

- 对一个模型(如 `gpt-5`)配置多个 provider(如 OpenRouter、硅基流动、DeepSeek 中转)。
- 当前 provider 不可用(429/5xx/超时/额度耗尽)时,自动切换到下一个可用 provider,CLI 侧无感知(或仅一次重试)。
- 旧实现对标:用户手册 `4.3-failover.md` 的故障转移队列 + 熔断器(阈值/半开/恢复),按**应用**维护队列;R1 是把它细化为按**模型**维护队列。

### R2:指针式配置,替代复写式切换

- 用户对现状的不满:"CC Switch 貌似是复写配置的,非常难用,经常会出错。"
- 直觉:写多个配置文件,切换时只把指针指过去,不整体复写。
- 约束(本草案的分析):Claude Code / Codex / Gemini CLI **都没有 include/symlink 配置语义**,不能真的"指到另一个文件"。可用的间接层只有两种:
  1. **base URL 指针**:`ANTHROPIC_BASE_URL`(Claude)、`model_providers.*.base_url`(Codex)、`GEMINI_BASE_URL`(Gemini)——全部指向本地路由;
  2. **选择器字段**:`model_provider = "..."`(Codex)。
- 结论:把"指针"指向**本地路由**这一个稳定目标,provider 切换在路由内部完成。CLI 配置在初始化时写一次,之后**永远不改**。
- **启动路径约束(2026-08-13 查证)**:用户的启动方式是固定命令(终端)、点图标(GUI)、IDE 唤起——三种方式都**不携带参数**,唯一共同读取的是配置文件。因此:
  - 配置文件指针(写一次)是唯一覆盖全部启动方式的方案,定为**主线**;
  - "启动时递纸条"(`claude --settings ...`、`ANTHROPIC_BASE_URL=... claude` 等)只覆盖终端场景,且要求用户改启动方式——**降级为可选增强,不进 v1**;
  - PATH 替身(包装脚本代递参数)只覆盖终端,不覆盖 GUI——同样降级;
  - 真实先例:claude-code-router 走"本地网关 + 稳定端点"(即写一次指针,成熟方案);claude-code-proxy 走"启动时环境变量"(要求改启动命令,正是不适合本用户的形态);
  - 另发现:Codex 第一方支持多档案配置文件(`$CODEX_HOME/<name>.config.toml`,官方 config-reference),选择器形态待验证(D10)。

### R3:Codex 官方登录 + 本地换 provider

- 保留 `auth.json` 里的官方 ChatGPT / Codex 登录态(Codex App 的远程操作、官方插件依赖它)。
- 模型流量经本地路由走第三方 provider。
- 旧实现对标:guides/codex-official-auth-preservation-guide-zh.md(v3.16.1 起"Codex 应用增强")——第三方信息只写 `config.toml`(owned 字段),`auth.json` 不动。**与 2026-07-16 切换设计的 Codex API 模式完全一致**("API switching never overwrites auth.json")。

### R4:合买订阅 + VPS 反代(远端共享拓扑)

- 未来场景:几个朋友合买一个订阅账号,租一台新加坡/马来西亚/日本 VPS 做反代把 API 接出来;本地的订阅 + 中转 + 官方 API 维持直连。
- 2026-08-13 用户拍板(与本地路由正交、但决定 VPS 上游形态的拓扑决策):
  1. **VPS v1 = 纯反代**(Caddy / Nginx 注入共享订阅凭证),不做路由决策、不常驻计量服务;
  2. **计量放客户端**:共享订阅只有一份账号级配额,每个朋友各自用本 app 监控同一账号;或平摊不计费;
  3. **LiteLLM 仅作触发式升级**:出现"一人滥用拖垮全队 / 连累共享账号被风控"或"按用量分账"需求时,才在 VPS 上升级为 LiteLLM 网关(虚拟 key + budget + spend),**仍不开它的 router/fallback**——路由决策始终在本地;
  4. 本地 router 眼里,VPS 反代只是 provider 队列里的普通逻辑 provider(可按订阅拆成 Claude-共享 / GPT-共享)。
- 待验证:反代上游的协议形态与凭证形态(见 Q8/Q9)。

---

## 2. 历史与现状

### 2.1 产品线沿革

- 本仓库是 CC Switch 产品线的延续:用户手册标题即"CC Switch 用户手册",版本线连续到 3.16.x,git 历史存在 `feat(app): isolate dashboard from cc switch` 等隔离/重命名提交。现以 **LLM Usage Bar** 名义运行。
- **2026-07-17 pivot**(provider-only-monitoring-design.md,已批准):app 定位为**只读的 Provider/账号监控**;不做切换、不做代理;`CC Switch owns Provider switching`。
- **schema v21**(合入 main):删除九张退役表 `mcp_servers`、`prompts`、`profiles`、`provider_health`、`skills`、`skill_repos`、`proxy_config`、`proxy_live_backup`、`stream_check_logs`。
- 当前 `HANDOFF.md:625` 明确:"这个代码库里**根本不存在本地 proxy**"。

### 2.2 被删除的旧实现(有文档、无代码)

用户手册 4-proxy 四章(`4.1-service` / `4.2-routing` / `4.3-failover` / `4.4-usage` / `4.5-model-test`)与 guides 完整记录了旧实现:

| 能力 | 旧实现要点 |
| --- | --- |
| 本地代理 | `127.0.0.1:15721`,按应用接管(Claude / Codex / Gemini) |
| 接管方式 | 改 CLI 配置指向代理,原配置备份到 `proxy_live_backup`,停止代理时恢复 |
| 故障转移 | 按应用维护供应商队列(拖拽排序);失败计数 → 熔断(阈值/半开/恢复);自动切下一个 |
| 熔断参数 | 通用:失败阈值 4、恢复阈值 2、恢复等待 60s、错误率 60%、最小请求数 10(Claude 有宽松默认) |
| 超时 | 流式首字节超时、流式静默超时、非流式超时(Claude 均有宽松默认) |
| 协议转换 | Codex 第三方 provider 为 Chat Completions 时:Responses ↔ Chat Completions 转换 |
| 官方登录保留 | v3.16.1 起:第三方信息只写 `config.toml` 的 owned 字段,`auth.json` 保留官方登录 |
| 请求日志 | `proxy_request_logs` 类表(注:该表不在 v21 删除清单内,HANDOFF.md:143 记录过 reviewer 的同类误判) |

### 2.3 2026-07-16 切换设计(被取代,未落地)

07-16 设计是更严谨的切换架构:owned-field 写入、字节级验证、journal/回滚、drift 检测、会话归因 epoch、Custom Agent 适配器、官方桥接。其明确 non-goal 包括 **"Automatic Provider failover, load balancing, or background optimization"**。

本草案与它的关系:**借用配置安全原则**(只写 owned 字段 + 验证 + 回滚,用于一次性指针设置),**不复活其全量切换体系**(按需改 CLI 配置的模型被 R2 的"一次性指针"取代)。

---

## 3. 旧实现的问题分析(用户不满的根因)

为什么"复写配置"会"非常难用、经常出错":

1. **复写式切换**:每次切换都整段改写 CLI 配置(依赖备份/恢复)。写入窗口、异常退出、与用户或外部工具并发编辑都会造成漂移;停止代理时的"恢复"本身又是一次整文件复写,出错面与切换相同。
2. **代理常驻 + 全量请求日志**:与低功耗目标冲突——tech-route-review 已把 1.62% 平均 CPU 当作问题(§2.3 实测),常驻代理和全量日志正是旧实现的耗电来源之一。
3. **熔断不感知配额**:健康状态只来自代理内请求统计;不知道订阅剩余窗口、不知道 API key 余额。可能把流量打到额度已耗尽的 provider,或在配额耗尽时反复失败。
4. **健康检查/恢复依赖轮询与备份**:旧实现有 `stream_check_logs`(流式健康检查)与 `proxy_live_backup`(live 备份),均为轮询/快照式机制,空闲时也在消耗资源。

---

## 4. 设计目标与原则

| 原则 | 内容 |
| --- | --- |
| **P1 请求驱动** | 路由服务只在有请求时工作。**无轮询健康检查**;provider 状态由失败反馈(冷却/熔断)驱动。与低功耗前提一致。 |
| **P2 一次性指针** | 初始化时把 CLI 指向本地路由(base URL 一次写入,走 07-16 的 owned-field + 验证 + 回滚)。运行期**不再修改任何 CLI 配置**;provider 切换全部发生在路由内部。 |
| **P3 配额感知** | 路由决策输入 = 现有监控数据(订阅剩余窗口、API key 余额、日预算)+ 失败反馈(冷却/熔断)。额度耗尽的 provider 自动跳过。 |
| **P4 归因闭环** | 路由记录"哪个请求由哪个 provider 实际服务",落库后与监控同源;用量面板能回答"每个 provider 实际承担了多少"。 |
| **P5 fail-closed** | 路由不可用 = CLI 明确报错,**不静默直连**。静默直连会让用户误判计费方、绕过配额监控(与 guides 中"不要用 Codex 账号信息判断计费方"的既有警告一致)。菜单栏显示路由健康。 |
| **P6 最小面** | 复用旧实现的协议转换与接管知识,但 v1 只覆盖用户实际使用的路径(Codex 优先),其余工具视验证结果逐步扩展。 |

---

## 5. 架构草图

### 5.1 组件

```
┌─ CLI(Codex / Claude Code / Gemini)──────────────────┐
│  base_url 指针(一次性写入)→ 127.0.0.1:<port>        │
└──────────────────────┬──────────────────────────────┘
                       ▼
┌─ 路由服务(loopback,请求驱动)────────────────────────┐
│  ① 模型 → provider 队列表(逻辑模型名,队列带优先级) │
│  ② 运行状态:per-provider 冷却 / 熔断 / 配额快照     │
│  ③ 路由决策:配额过滤 → 优先级 → 冷却/熔断跳过 → 转发│
│  ④ 协议转换(Codex Responses ↔ Chat Completions,可选)│
│  ⑤ 请求归因日志(落库)                               │
└──────────────┬─────────────────────┬────────────────┘
               ▼                     ▼
        第三方 provider API     配额/用量数据(现有监控侧)
        (upstream 凭证只存在    (订阅窗口 / key 余额 / 预算)
          protected 存储)        (共享订阅:客户端各自监控,见 R4)
        ▲ 上游含:官方 API / 中转(他人 VPS)/ 共享订阅 VPS 反代(纯 Caddy)
```

### 5.2 路由流程

```
CLI 请求 → 路由 → 取该 model 的队列
  → 过滤:额度不足 / 熔断中 / 冷却中 → 跳过
  → 按优先级转发到第一个可用 provider
      ├─ 成功(含首字节已流出)→ 正常返回,记录归因
      └─ 失败(429 / 5xx / 连接失败 / 首字节超时)
          → 记失败 → 更新冷却/熔断状态 → 队列下一个
          → 队列耗尽 → 返回聚合错误(fail-closed)
```

### 5.3 流式语义

- **首字节之前**:可自由切换 provider(失败重试下一个)。
- **首字节之后**:不可切换(已向客户端流出 token),只能终止并返回错误。这是流式协议的固有限制,旧实现同样如此(有"流式首字节超时""流式静默超时"两个检查点)。
- CLI 侧重试策略与路由侧重试(旧实现最大重试 3 次)的关系见 Q2。

### 5.4 数据流

- **路由输入**:现有配额收集器的输出(订阅剩余窗口、key 余额、日预算)——P3。
- **路由输出**:请求级归因事件(时间 / provider / model / 请求与响应 token / 结果)。新表设计见 D6,与 `usage_events` 的关系需对齐,避免出现"监控口径与路由口径不一致"。

---

## 6. 关键设计决策(需评审逐条拍板)

| # | 决策 | 草案建议 | 理由 |
| --- | --- | --- | --- |
| **D1** | 产品边界:撤销 07-17"只读"定位的哪一部分? | 只恢复"写路由自有状态 + 一次性指针";**不恢复**"按需反复改 CLI 配置"的 CC Switch 式切换模型 | R2 正是要消灭反复改写;一次性指针与 07-17 的冲突面最小 |
| **D2** | 与外部 CC Switch 的关系 | 自建路由,与 CC Switch 互不读写对方状态(延续 07-17"不打开对方数据库"原则) | 用户已表达其复写式切换不可靠;本方案是替代,不是协同 |
| **D3** | 路由不可用时的行为 | **fail-closed** + CLI 明确报错 + 菜单栏健康提示 | P5;静默直连会破坏配额监控与计费判断 |
| **D4** | 协议转换(Responses ↔ Chat Completions)是否进 v1 | **进**(Codex 第三方 provider 多为 Chat Completions) | 没有它 R3 对多数第三方 provider 不成立;旧实现有现成知识可复用 |
| **D5** | 模型→provider 队列的编辑形态 | 沿用旧式 UI(供应商队列拖拽排序 + 健康徽章),但入口挂在模型上 | 用户已熟悉旧交互;按模型比按应用更贴合 R1 |
| **D6** | 归因存储 | 新表(请求级:provider / model / token / 结果 / 失败原因),与 `usage_events` 口径对齐;命名不复用旧 `proxy_request_logs` | v21 删除后的重建要重新定义口径;避免旧"全量日志"的功耗问题(见 Q5) |
| **D7** | 路由服务生命周期 | 随 app 常驻,请求驱动,空闲零成本;启动时验证指针配置一致性 | P2 要求路由随时可用("指针永远指向它");低功耗由"无轮询 + 无全量日志"保证 |
| **D8** | 与 tech-route-review 的关系 | 路由属核心层,与 Swift/Rust 选型正交;核心若迁 Swift,路由同样在核心实现 | 避免在语言评审前绑死技术栈 |
| **D9** | 启动器层(启动参数 / PATH 替身)进 v1? | **不进,降级为可选增强** | 用户启动命令固定,GUI 与 IDE 不携带参数也不走 PATH;配置文件指针是唯一全覆盖层(§1 R2 启动路径约束) |
| **D10** | Codex 第一方 profile 文件机制(`$CODEX_HOME/<name>.config.toml`)是否替代自管 config.toml owned 字段? | 阶段 3 评估;若选择器形态验证可行,优先采用 | 官方原生多档案,写入面比自管字段更小;选择器形态(flag/env)未在文档确认,实现前验证 |

---

## 7. 范围与 Non-goals

### v1 范围(建议)

- Codex 优先:一次性指针 + 单 provider 转发 + 请求归因 + fail-closed(阶段 1);
- 多 provider 队列 + 冷却/熔断 + 配额过滤(R1,阶段 2);
- Responses ↔ Chat Completions 转换 + 官方登录保留验证(R3,阶段 2/3);
- Claude / Gemini 接管视 Codex 实测结果决定(阶段 3)。

### Non-goals

- **不缝 LiteLLM(本地与远端 v1)**:本地——Python 常驻服务,包体、功耗、双工具链与低功耗前提冲突;本仓库历史已证明该功能在 Rust/TS 栈内完整可行。远端 v1——VPS 只做纯反代(Caddy),不上 LiteLLM;仅在 §1 R4 触发条件满足时作为网关升级选项,且定位是网关(虚拟 key + 限额),不是 router。
- **不做负载均衡 / 流量镜像 / 多机同步路由状态**:R1 只要求故障转移,不要求流量分配。
- **不做 CC Switch 式切换模型**:不按需反复改写 CLI 配置。
- **不做 MCP / prompts / skills 管理**:与 v21 删除保持一致,不借本方案复活。
- **不做轮询健康检查**:状态只来自失败反馈与现有配额收集器(P1)。

### 已查证的第三方事实(2026-08-13 官方文档)

- **Claude Code**:官方 llm-gateway 文档原文 "Anthropic doesn't support routing Claude Code to non-Claude models through any gateway" ——跨 provider 路由对 Claude Code 是**灰色地带**(技术可行、官方不背书);对 Codex / Gemini 无此限制。R1 对 Claude Code 的定位:可行但用户自担。
- **Gemini CLI**:自带模型级自动 fallback(ModelAvailabilityService,默认开启),但仅限 Gemini 自家模型之间,不跨 provider;`GEMINI_BASE_URL` 官方文档未见(CC Switch 旧文档声称有,未验证)。
- **Codex 监控侧机会**:Codex 提供第一方只读 RPC(`codex -s read-only -a untrusted app-server`,暴露 `account/read`、`account/rateLimits/read`)——监控侧读额度可优先使用该通道,与路由无关(CodexBar 同款做法,纯被动)。

---

## 8. 落地阶段(若批准)

| 阶段 | 内容 | 出口条件 |
| --- | --- | --- |
| **1. 最小路由** | Codex 一次性指针(owned-field + 验证 + 回滚)、loopback 转发、请求归因落库、fail-closed | Codex 走本地路由完成一次真实请求;归因出现在用量面板;指针写入/回滚有测试 |
| **2. 故障转移** | 模型→provider 队列、冷却/熔断、配额过滤 | R1 端到端:主 provider 故障时自动切备用,配额耗尽自动跳过;熔断参数可配 |
| **3. 协议与扩展** | Responses ↔ Chat Completions 转换、Claude/Gemini 接管、队列编辑 UI、健康徽章、菜单栏路由状态;评估 Codex 第一方 profile 机制(D10)与启动器层可选增强 | R3 实测通过(官方登录保留 + 插件/远程可用);全部工具有等价测试 |

每阶段独立评审;决策与进展按仓库约定记录进 `HANDOFF.md`(新工作不另开顶层文档)。

---

## 9. 开放问题(评审时逐条回答)

| # | 问题 | 备注 |
| --- | --- | --- |
| **Q1** | Codex 官方登录 + 自定义 provider 下,插件与远程操作是否完全可用? | 必须实测;guides/codex-official-auth-preservation-guide-zh.md 有现成验证步骤可复用;顺带验证 Codex profile 选择器形态(D10) |
| **Q2** | 失败后由谁重试:CLI 侧重试 vs 路由侧重试?各重试几次? | 旧实现路由侧重试 3 次;CLI 自身可能也会重试,叠加会放大延迟 |
| **Q3** | 冷却/熔断参数:沿用旧默认(4 次 / 60s / 半开 2 次)还是更保守? | 旧默认有完整文档与最佳实践(4.3-failover.md) |
| **Q4** | 配额"不足"的定义:剩余窗口 / key 余额 / 日预算?订阅 provider 与 metered provider 混排队列的规则? | 直接复用 07-17 的预算与配额语义,避免新造口径 |
| **Q5** | 归因日志保留策略:只保留失败事件 + 汇总,还是限行数滚动? | 直接决定功耗;旧"全量日志"是问题来源之一(P1) |
| **Q6** | 菜单栏/面板呈现:路由健康、当前 provider、failover 事件怎么展示? | 与"左键弹窗仅用量"的既有约束协调(07-16 设计曾明确左键弹窗不做配置动作) |
| **Q7** | 用户实际启动方式清单(终端固定命令 / Codex 桌面 / IDE / 远程)是否全部经配置文件生效? | 决定 D9;若存在不读配置文件的启动方式,需补机制 |
| **Q8** | 合买订阅反代暴露的协议形态(Anthropic Messages / OpenAI Responses / Chat Completions)与凭证形态(订阅 OAuth token / API key)? | 决定本地 router 对该 upstream 是否需要协议转换(D4 范围)与 Caddy 反代的 path/header 配置 |
| **Q9** | 共享订阅的配额对客户端是否可见(客户端侧计量成立的前提)? | 若账号级配额对客户端不可见,则需回退到服务器侧计量(LiteLLM 网关升级) |

---

## 10. 一句话总结

**在 Rust 核心内重建一个请求驱动的轻量本地路由**:CLI 配置一次性指向它(消灭复写;用户启动命令固定,配置文件是唯一全覆盖的注入口),模型→provider 队列 + 冷却/熔断 + 配额过滤(故障转移),请求归因落库(监控闭环),Codex 官方登录原样保留(生态不变)。功能面是"恢复 v21 删除的子集",架构面是对旧实现失败模式的修正,数据面是本 app 独有的配额感知。无需 LiteLLM。
