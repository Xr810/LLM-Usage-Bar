# 用量仪表盘后端优先改造计划

## Implementation Status

**Complete as of 2026-07-12.** Tasks 0–8 are implemented, independently reviewed, committed and verified with the repository-pinned Rust 1.95 toolchain plus the complete frontend gate. PR #6's initial backend CI run exposed two `-D warnings` Clippy findings omitted from the local final gate; commit `67654d44` fixes both without lint suppression and passes the exact CI Clippy command plus the full Rust gate locally. The optional explicit read-only import from original CC Switch remains intentionally deferred and is not required for coexistence.

## 开发上下文

- 当前唯一开发仓库：`/Users/max/LLM Usage Bar`（远程：`Xr810/LLM-Usage-Bar`）。
- 当前隔离工作区：`/Users/max/LLM Usage Bar/.worktrees/codex-usage-dashboard-backend`，分支 `codex/usage-dashboard-backend`。
- 旧路径 `/Users/max/CCSwitchUsageDashboard` 是此前 CC Switch fork 的工作区，**不再作为本计划的开发目标**。

## Summary

先建立独立、可信的用量后端闭环，再接入基础前端，最后裁剪旧 CC Switch 模块。

- 里程碑 1：数据库迁移、Provider 计费模型、UsageEvent、静态代理路由、额度快照、Session 导入和聚合 API。
- 里程碑 2：基础仪表盘、Provider/路由配置和手动刷新能力。
- 里程碑 3：确认新链路稳定后，分批移除切换、故障转移、MCP、Skills、OpenClaw、云同步等旧模块。
- 过渡期保留旧表和旧日志双写；至少一个稳定版本内不执行破坏性 DROP。

## Backend Changes

### 1. 数据模型与迁移

将数据库从 schema v12 升级到 v13，新增：

- `usage_providers`：全局唯一 Provider，包含 `billing_kind`、`product_group_id`、`token_sources`、quota/routing 配置和启用状态。
- `route_bindings`：以 `claude | codex | gemini` 为主键，每个协议只能绑定一个启用的 metered Provider。
- `usage_source_bindings`：将 Claude/Codex session 来源显式绑定到 Provider。
- `usage_events`：不可变逐请求记录，包含 Token、模型、来源、关联 ID、分项费用和 `cost_source`。
- `usage_event_links`：保存基于稳定 ID 的跨来源重复关系，不修改或删除原始事件。
- `quota_snapshots`：只追加成功额度快照。
- `quota_fetch_state`：保存最后尝试、最后成功、错误和过期状态。

迁移规则：

- 可识别的 Codex OAuth、GitHub Copilot、Coding Plan Provider 迁移为 subscription；其余旧代理 Provider 默认为 metered。
- 无法确定计费类型的 Provider 标记为需要检查，不静默归类为订阅。
- 旧 proxy 日志映射到新事件时标记 `cost_source=estimated`。
- 旧 session 占位日志若无法精确映射 Provider，不导入新统计，但保留在旧表。
- 复用现有迁移前数据库备份和未来版本拒绝机制。

### 2. 用量采集与费用可信度

建立统一 `UsageIngestionService`：

- 扩展响应解析器，优先读取上游明确返回的费用字段。
- 有上游费用时使用 `upstream`；无上游费用但存在定价时使用 `estimated`；两者都没有则使用 `unavailable`。
- 费用使用十进制定点字符串存储，分项未知时保持为空，不伪造为真实零费用。
- 新事件、关联关系和过渡期旧日志在同一 SQLite 事务中写入。
- 新事件使用幂等插入，不再沿用 `INSERT OR REPLACE` 覆盖历史事件。
- 日志落库失败不改变已经成功的上游响应，但记录诊断错误并通知前端。

跨来源去重只允许同一 Provider 的 proxy 与 session 使用完全相同的稳定 request/session/upstream correlation ID。删除新查询链路中的“模型、Token 数、相近时间”指纹去重；没有稳定证据的事件分别计数并保留来源标记。

### 3. 静态代理路由

将代理运行时选路改为读取 `route_bindings`：

- 每个入站协议只有一个固定 Provider，修改绑定后从下一次请求生效。
- 保留现有单监听端口、协议转换、认证注入、流式转发和 Token parser。
- 绑定不存在、Provider 被禁用、类型不是 metered 或路由配置不完整时返回明确的本地 503。
- 不尝试 current provider、failover queue、熔断回退或其他 Provider。
- 首轮只让旧选路代码退出请求主链；物理删除放到里程碑 3。

### 4. 订阅额度与 Session Token

建立 provider-aware 的采集服务：

- 将现有 Claude、Codex、Coding Plan 查询封装成 `QuotaCollector` adapter。
- 默认每 5 分钟检查一次，可按 Provider 配置；失败等下个周期或手动刷新，不进行密集重试。
- 成功追加快照；失败只更新 `quota_fetch_state`，最后成功数据继续展示并标记过期/失败。
- 支持可选的手动重置剩余次数，来源不支持时返回空值。
- Session 导入必须先存在明确的 `usage_source_bindings`；未绑定来源跳过并返回可见警告。
- QuotaSnapshot 永远不写入 UsageEvent，也不参与 Token、费用或每日趋势聚合。

## Public Interfaces and Basic Frontend

新增核心类型：

- `BillingKind = subscription | metered`
- `TokenSource = proxy | session_log`
- `CostSource = upstream | estimated | unavailable`
- `UsageProvider`、`RouteBinding`、`UsageEvent`、`QuotaSnapshot`
- `ProductUsageView`：后端已经按产品分组的仪表盘 DTO

新增 Tauri 命令：

- `list_usage_providers`
- `save_usage_provider`
- `set_usage_provider_enabled`
- `get_route_bindings`
- `set_route_binding`
- `get_usage_dashboard(startAt, endAt, productGroupId?)`
- `get_usage_events(providerId, startAt, endAt, page, pageSize)`
- `refresh_provider_quota(providerId)`
- `sync_provider_session_usage(providerId)`

API 不返回明文凭据；列表接口仅返回脱敏路由信息。

基础前端仅实现产品分组与 Token 来源说明、subscription 卡、metered 卡、简化 Provider 配置、静态绑定、代理启停、额度刷新和 Session 同步。首轮隐藏旧功能入口，不投入复杂图表和视觉优化。

## Test Plan

后端必须覆盖：

- v12 → v13 迁移、旧数据保留、重复执行幂等和迁移失败回滚。
- 上游费用、估算费用、费用不可用三种写入路径。
- 有稳定关联 ID 时只统计一次；无 ID 或仅时间/Token 相似时分别统计。
- RouteBinding 正确选路，未绑定返回 503，并确认不会触发故障转移。
- Quota 查询失败后最后成功快照保持不变。
- 未绑定 Session 来源不导入，显式绑定后归属正确。
- 同一产品下 subscription 与 metered Provider 分开聚合。
- QuotaSnapshot 不进入 Token 或费用汇总。
- 分页、时间范围、Provider 和 product group 查询边界。

前端只测试基础数据渲染、费用来源标签、过期额度状态、时间筛选和路由配置。

每个里程碑运行：

- `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `pnpm typecheck`
- `pnpm test:unit`
- `pnpm build:renderer`

最终使用本地 mock upstream 完成一次真实代理请求验收，确认它生成可按 Provider 和时间范围查询的 UsageEvent。

## Assumptions

- 每个 Claude/Codex/Gemini 入站协议首版只允许一个 metered RouteBinding。
- 费用允许真实值与估算值并存，但必须明确标注，不能混成一个无来源总数。
- 首轮采用迁移、双写、验证后裁剪的策略，不同时进行全仓库大删除。
- 前端目标是可配置、可查询、可诊断，不以 UI 完成度作为首轮验收重点。
