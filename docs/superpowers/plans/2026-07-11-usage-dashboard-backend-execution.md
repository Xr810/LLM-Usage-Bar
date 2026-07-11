# Usage Dashboard Backend-First Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有 Tauri/React 应用上建立 provider-aware 的可信用量闭环：静态代理路由、不可变逐请求事件、独立订阅额度快照、显式 Session 归属、产品分组查询和最小可用前端。

**Architecture:** 新的 `usage` 领域以全局 `usage_providers` 为账户边界，以 `route_bindings` 和 `usage_source_bindings` 显式决定代理与 Session 归属。代理响应统一进入 `UsageIngestionService`，在同一 SQLite 事务内幂等写入新事件、稳定 ID 关联和过渡期旧日志；额度只写 quota 表。现有协议转换、认证和流式转发保留，但请求主链只接受一个启用的 metered 静态绑定。

**Tech Stack:** Rust 1.95（由 `rust-toolchain.toml` 固定；`Cargo.toml` 的 `rust-version=1.85.0` 是最低版本）、Tauri 2、rusqlite 0.31、rust_decimal、Tokio、Axum、React 18、TypeScript 5、TanStack Query、Vitest/Testing Library、SQLite。

## Global Constraints

- 当前唯一开发仓库：`/Users/max/LLM Usage Bar`；执行工作区：`/Users/max/LLM Usage Bar/.worktrees/codex-usage-dashboard-backend`。
- schema 从 `12` 升到 `13`；继续复用迁移前备份、SAVEPOINT 回滚和未来版本拒绝机制。
- v13 只新增表、索引和双写，不 `DROP` 旧表；本计划的里程碑 3 只让旧能力退出请求主链并隐藏入口，不物理删除模块。
- `BillingKind = subscription | metered`；`TokenSource = proxy | session_log`；`CostSource = upstream | estimated | unavailable`。
- 金额使用十进制定点字符串；未知分项存 `NULL`，不得伪造为真实零费用。
- `usage_events` 不可变且幂等；不得使用 `INSERT OR REPLACE` 覆盖历史事件。
- 跨来源去重只允许同一 Provider 上完全相同的稳定 request/session/upstream correlation ID；禁止按模型、Token 或时间近似去重。
- `QuotaSnapshot` 不得写入 `UsageEvent`，不得进入 Token、费用或每日趋势聚合。
- Tauri 查询接口不返回明文凭据；只返回脱敏路由信息和凭据是否存在。
- 静态绑定从下一次请求生效；未绑定、禁用、非 metered 或路由不完整返回本地 HTTP 503，且不得尝试 current provider、failover queue 或其他 Provider。
- Cargo 验证统一使用仓库 `rust-toolchain.toml` 的 Rust 1.95；已知可用验证环境为 `/tmp/codex-usage-dashboard-rust`，每个 Task 仍须在对应提交上重新执行自己的 Cargo 门槛。
- 每个 Task 独立走 Red-Green-Refactor、完整验证和单独提交；不要把多个 Task 合并成一个提交。

## File Structure

- `src-tauri/src/usage/domain.rs`: v13 枚举、实体、命令输入和公开 DTO。
- `src-tauri/src/usage/migration.rs`: 旧 Provider 分类与 v12 数据映射纯函数。
- `src-tauri/src/usage/ingestion.rs`: 费用可信度、稳定 ID 关联和事务写入编排。
- `src-tauri/src/usage/quota.rs`: quota adapter、快照/失败状态与调度。
- `src-tauri/src/usage/session.rs`: provider-aware Claude/Codex Session 同步入口。
- `src-tauri/src/usage/dashboard.rs`: 产品分组、范围和分页查询。
- `src-tauri/src/database/dao/{usage_providers,usage_events,quota}.rs`: 三组 v13 DAO。
- `src-tauri/src/commands/usage_dashboard.rs`: 九个新 Tauri 命令。
- `src/{types,lib/api,lib/query}/usageDashboard.ts`: 前端 wire contract、API 与 hooks。
- `src/components/usage-dashboard/`: 产品组、两类 Provider 卡、Provider/路由配置和页面。

---

### Task 0: Isolate LLM Usage Bar from Original CC Switch

**Status:** Complete and independently reviewed (`e931e7f9`, `7fb6a6df`, `ca38e0ef`). Optional explicit read-only snapshot import remains deferred; the dashboard never auto-opens or upgrades the original v11 database.

- [x] Use a distinct product name, bundle identifier, updater policy, data/settings directory, and default proxy port; register no legacy deep-link scheme.
- [x] Reject configuration-directory overrides that point back to the original `~/.cc-switch` data.
- [x] Verify the original v11 database is unchanged by all dashboard tests and startup preparation.
- [ ] If original data import is added, require explicit user action and use a read-only SQLite Backup snapshot into the isolated target; never copy or migrate the live source database in place.
- [x] Document that both apps may run concurrently but may not both control the same Claude/Codex/Gemini live configuration.

### Task 1: Define the Domain and Migrate SQLite from v12 to v13

**Status:** Complete and independently reviewed (`55814021`, `1fcaf35d`).

**Files:**
- Create: `src-tauri/src/usage/domain.rs`
- Create: `src-tauri/src/usage/migration.rs`
- Create: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Test: `src-tauri/src/usage/domain.rs`
- Test: `src-tauri/src/database/tests.rs`

**Interfaces:**
- Consumes: old `providers`, `proxy_request_logs`, current migration runner.
- Produces: public domain types; `SCHEMA_VERSION = 13`; seven v13 tables; `classify_legacy_provider(...) -> LegacyProviderClassification`.

- [ ] **Step 1: Write failing domain and migration tests**

Add tests that:

- deserialize only the three documented enum value sets;
- reject empty provider ID/name/product group and quota intervals 1-59 seconds;
- migrate a true v12 fixture containing Codex OAuth, GitHub Copilot, token-plan, ordinary metered and ambiguous official Providers;
- preserve every old Provider/log row;
- import only proxy logs into `usage_events` with `cost_source='estimated'`;
- skip `_session` placeholders that cannot be mapped;
- set `needs_review=1` for ambiguous official rows;
- are idempotent on a second migration call;
- roll back tables, rows and `user_version` when a forced trigger fails.

Core assertions:

```rust
assert_eq!(Database::get_user_version(&conn).unwrap(), 13);
assert_eq!(count(&conn, "providers"), legacy_provider_count);
assert_eq!(count(&conn, "proxy_request_logs"), legacy_log_count);
assert_eq!(scalar_text(&conn, "SELECT cost_source FROM usage_events LIMIT 1"), "estimated");
assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM usage_events WHERE provider_id='_session'"), 0);
```

- [ ] **Step 2: Run tests and verify the expected failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage::domain::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml migration_v12_to_v13 -- --nocapture
```

Expected: compilation/test failure because the module, types and v13 migration do not exist.

- [ ] **Step 3: Implement the domain contract**

Define snake_case enums and camelCase structs:

```rust
pub enum BillingKind { Subscription, Metered }
pub enum TokenSource { Proxy, SessionLog }
pub enum CostSource { Upstream, Estimated, Unavailable }

pub struct UsageProviderInput {
    pub id: String,
    pub name: String,
    pub billing_kind: BillingKind,
    pub product_group_id: String,
    pub token_sources: Vec<TokenSource>,
    pub quota_source: Option<String>,
    pub quota_interval_seconds: Option<u64>,
    pub route_app_type: Option<String>,
    pub route_config: Option<serde_json::Value>,
    pub quota_config: Option<serde_json::Value>,
    pub enabled: bool,
}
```

Add stored/public Provider variants so `UsageProviderView` exposes only `route_base_url` and `has_route_credentials`. Define `RouteBinding`, `UsageSourceBinding`, immutable `UsageEvent`, `UsageEventLink`, `QuotaSnapshot`, `QuotaFetchState`, `UsageEventPage`, `ProductUsageView`, and `UsageDashboardView`. Cost fields and stable IDs are `Option<String>`.

- [ ] **Step 4: Implement the atomic v13 migration**

Create these tables and indexes in `migrate_v12_to_v13` under the existing SAVEPOINT:

- `usage_providers`: global ID, billing kind, product group, JSON token sources, quota/routing configs, enabled/review flags, transitional legacy app/provider reference.
- `route_bindings`: protocol PK restricted to `claude|codex|gemini`, one Provider FK.
- `usage_source_bindings`: source PK restricted to `claude|codex`, one Provider FK.
- `usage_events`: immutable event PK, Provider/product/time/model/tokens, three stable IDs, nullable cost parts, `cost_source`, unique legacy request ID.
- `usage_event_links`: canonical/duplicate event IDs, exact link kind/value, composite PK.
- `quota_snapshots`: append-only normalized windows plus raw payload.
- `quota_fetch_state`: last attempt/success/error/stale by Provider.

Add indexes `(provider_id, occurred_at DESC)`, `(product_group_id, occurred_at DESC)`, stable IDs, and quota Provider/time.

Classification is deterministic: `meta.provider_type` equal to `codex_oauth` or `github_copilot`, or enabled usage script template `official_subscription`/`token_plan`, becomes subscription; all other proxy Providers default metered; `category='official'` without proof also sets `needs_review=1`. Legacy IDs become `{app_type}:{provider_id}`.

- [ ] **Step 5: Run the database gate and commit**

Run:

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml database::tests -- --nocapture
```

Expected: all old migration tests and new v13 tests pass.

```bash
git add src-tauri/src/usage src-tauri/src/database/mod.rs src-tauri/src/database/schema.rs src-tauri/src/database/tests.rs src-tauri/src/lib.rs
git commit -m "feat(db): add provider-aware usage schema"
```

### Task 2: Implement v13 Persistence for Providers, Bindings, Events, and Quotas

**Status:** Complete and independently reviewed (`ac70ec35`, `18e146fe`).

**Files:**
- Create: `src-tauri/src/database/dao/usage_providers.rs`
- Create: `src-tauri/src/database/dao/usage_events.rs`
- Create: `src-tauri/src/database/dao/quota.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Test: each created DAO module

**Interfaces:**
- Consumes: Task 1 types/tables.
- Produces: Provider/binding CRUD; immutable event insert/link/query; append-only quota success and failure-state methods.

- [ ] **Step 1: Write failing DAO tests**

Cover:

- global Provider ID uniqueness and secret redaction;
- Provider UPSERT without losing stored credentials when edit payload omits them;
- route binding accepts only enabled metered Provider;
- source binding accepts only a Provider whose `token_sources` contains `session_log`;
- duplicate event ID returns `inserted=false` and does not overwrite the original;
- stable match requires same Provider and exact non-empty identifier;
- pagination is deterministic and enforces page size 1-200;
- quota failure preserves the last successful snapshot and only marks fetch state stale.

Key assertions:

```rust
assert!(!serde_json::to_string(&public_provider).unwrap().contains("secret"));
assert_eq!(db.set_route_binding("claude", "subscription-id").unwrap_err().to_string(), "route provider must be metered");
assert!(!db.insert_usage_event(&changed_duplicate).unwrap());
assert_eq!(db.latest_quota_snapshot("sub").unwrap().unwrap().captured_at, first_success_at);
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml database::dao -- --nocapture`

Expected: v13 DAO methods are undefined.

- [ ] **Step 3: Implement Provider and binding DAO**

Add methods on `Database`:

```rust
list_usage_providers() -> Result<Vec<UsageProviderView>, AppError>
get_usage_provider(id: &str) -> Result<Option<UsageProvider>, AppError>
save_usage_provider(input: &UsageProviderInput) -> Result<UsageProviderView, AppError>
set_usage_provider_enabled(id: &str, enabled: bool) -> Result<(), AppError>
get_route_bindings() -> Result<Vec<RouteBinding>, AppError>
set_route_binding(protocol: &str, provider_id: &str) -> Result<RouteBinding, AppError>
get_usage_source_binding(source_key: &str) -> Result<Option<UsageSourceBinding>, AppError>
set_usage_source_binding(source_key: &str, provider_id: &str) -> Result<UsageSourceBinding, AppError>
```

Use explicit `INSERT ... ON CONFLICT DO UPDATE`, never `OR REPLACE`. Validate target type/enabled/source in the same locked connection as the binding UPSERT. Centralize redaction in one conversion function.

- [ ] **Step 4: Implement immutable event and quota DAO**

Event insert uses `ON CONFLICT(event_id) DO NOTHING`. Exact matching SQL always includes `provider_id=?` and one non-empty equality over request/session/upstream ID. A link keeps both events intact and is created only across different sources. Event list uses half-open time range and `ORDER BY occurred_at DESC, event_id DESC`.

Successful quota refresh appends one snapshot and UPSERTs success state. Failure never modifies `quota_snapshots`; it only updates attempt/error/stale while preserving `last_success_at`.

- [ ] **Step 5: Run tests and commit**

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage_providers::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml usage_events::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml quota::tests -- --nocapture
git add src-tauri/src/database/dao
git commit -m "feat(usage): persist providers events and quotas"
```

### Task 3: Replace Current/Failover Selection with One Static Route

**Status:** Complete and independently reviewed (`f323ea75`, `73ccb294`, `4fa50dea`, `0e27e91c`). The real proxy acceptance proves local 503/zero upstream hits even with reachable legacy fallback candidates.

**Files:**
- Modify: `src-tauri/src/proxy/provider_router.rs`
- Modify: `src-tauri/src/proxy/handler_context.rs`
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Modify: `src-tauri/src/proxy/error.rs`
- Modify: `src-tauri/src/proxy/error_mapper.rs`
- Test: `src-tauri/src/proxy/provider_router.rs`
- Test: `src-tauri/src/proxy/handler_context.rs`

**Interfaces:**
- Consumes: `Database::get_route_binding`, transitional `legacy_app_type/legacy_provider_id` bridge.
- Produces: `ProviderRouter::select_bound_provider(protocol) -> Result<Provider, AppError>`; one-attempt request path.

- [ ] **Step 1: Replace routing tests with failing static-binding cases**

Seed old current A and failover C, but bind usage Provider B. Assert only B is selected. Add missing binding, disabled target, subscription target and incomplete route config cases; assert each becomes a local 503 and the mock upstream receives zero requests.

- [ ] **Step 2: Run tests and verify the old behavior fails the new contract**

Run: `cargo test --manifest-path src-tauri/Cargo.toml provider_router::tests -- --nocapture`

Expected: selector still reads current/failover state or the new method is absent.

- [ ] **Step 3: Implement static selection**

Add `ProxyError::{RouteNotBound, RouteProviderDisabled, RouteProviderNotMetered, RouteConfigIncomplete}` and map all to 503. On every request, load binding and Provider fresh, validate it, then resolve the transitional old `Provider` needed by existing protocol adapters. Do not call effective/current Provider, failover queue, circuit breaker, or switch manager.

Change `RequestContext::new` to hold `vec![bound_provider]`. Change forwarder to make exactly one upstream attempt with `max_retries=0`; retain existing protocol conversion, auth injection, model mapping, headers, streaming and timeout behavior.

- [ ] **Step 4: Prove no fallback occurs**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml provider_router -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml proxy:: -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test proxy_commands -- --nocapture
rg -n "get_effective_current_provider|get_failover_queue|allow_provider_request" src-tauri/src/proxy/provider_router.rs src-tauri/src/proxy/handler_context.rs src-tauri/src/proxy/forwarder.rs
```

Expected: tests pass; final `rg` returns no request-path match.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/proxy/provider_router.rs src-tauri/src/proxy/handler_context.rs src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/error.rs src-tauri/src/proxy/error_mapper.rs
git commit -m "feat(proxy): enforce static provider routes"
```

### Task 4: Build Transactional Usage Ingestion and Capture Upstream Cost

**Status:** Complete and independently reviewed (`7fafece4`, `73ccb294`).

**Files:**
- Create: `src-tauri/src/usage/ingestion.rs`
- Create: `src-tauri/src/proxy/usage/cost_parser.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/proxy/usage/mod.rs`
- Modify: `src-tauri/src/proxy/usage/logger.rs`
- Modify: `src-tauri/src/proxy/response_processor.rs`
- Modify: `src-tauri/src/proxy/handlers.rs`
- Modify: `src-tauri/src/usage_events.rs`
- Test: created modules and `response_processor.rs`

**Interfaces:**
- Consumes: Task 2 DAO, existing Token parsers and model pricing.
- Produces: `UsageIngestionService::ingest`; `extract_upstream_cost`; proxy response integration and diagnostic event.

- [ ] **Step 1: Write failing trust and transaction tests**

Cover:

- explicit upstream total/parts win and set `upstream`;
- no upstream cost plus known pricing sets `estimated`;
- neither source sets `unavailable` with all cost fields `NULL`;
- explicit upstream numeric zero remains real `Some("0")`;
- duplicate insert cannot overwrite;
- one forced legacy-log failure rolls back event and link;
- same-Provider proxy/session records with exact stable ID create one link;
- same tokens/model/time without ID remain separate and unlinked;
- ingestion failure does not change the already successful upstream response.

- [ ] **Step 2: Run tests and verify failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage::ingestion -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml cost_parser -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml response_processor -- --nocapture
```

Expected: service/parser are absent and old logger writes only `proxy_request_logs`.

- [ ] **Step 3: Implement cost extraction and decision**

Parse only explicit upstream JSON fields such as `usage.cost`, `usage.total_cost` and documented cost-detail fields; accept JSON number/string through `Decimal`, reject negative/non-numeric values, and preserve missing parts as `None`.

Define input/output:

```rust
pub struct UsageIngestionInput {
    pub event_id: String,
    pub source: TokenSource,
    pub provider_id: String,
    pub occurred_at: i64,
    pub model: String,
    pub usage: TokenUsage,
    pub upstream_cost: Option<UpstreamCost>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub upstream_correlation_id: Option<String>,
    pub legacy: Option<LegacyLogInput>,
}
```

Decision order is upstream, estimated pricing, unavailable. `usage.dedup_request_id()` may generate the local event ID, but a random fallback must never become stable cross-source evidence.

- [ ] **Step 4: Implement the single transaction and response wiring**

Within one `rusqlite::Transaction`: validate Provider/source, insert event immutably, find/link exact stable duplicate, insert compatibility `proxy_request_logs` using `ON CONFLICT DO NOTHING`, commit, then notify UI. Refactor `UsageLogger` into a compatibility wrapper over ingestion.

For non-streaming responses pass response `id` as upstream correlation ID; for SSE inspect collected usage/terminal events. On failure keep the upstream response unchanged, log `[USG-001]`, and emit `usage-ingestion-error` with redacted Provider/request/message fields.

- [ ] **Step 5: Run proxy gate and commit**

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage::ingestion -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml cost_parser -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml response_processor -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml proxy:: -- --nocapture
git add src-tauri/src/usage src-tauri/src/proxy/usage src-tauri/src/proxy/response_processor.rs src-tauri/src/proxy/handlers.rs src-tauri/src/usage_events.rs
git commit -m "feat(usage): ingest trusted request costs"
```

### Task 5: Add Provider-Aware Quota Collection and Session Import

**Status:** Complete and independently reviewed (`fc57e960`, `49907148`, `958a1cf6`).

**Files:**
- Create: `src-tauri/src/usage/quota.rs`
- Create: `src-tauri/src/usage/session.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/subscription.rs`
- Modify: `src-tauri/src/services/coding_plan.rs`
- Modify: `src-tauri/src/services/session_usage.rs`
- Modify: `src-tauri/src/services/session_usage_codex.rs`
- Test: created modules

**Interfaces:**
- Consumes: existing Claude/Codex/Coding Plan collectors, source bindings, JSONL parsers, ingestion service.
- Produces: `QuotaService::refresh_provider`; five-minute scheduler; `SessionUsageService::sync_provider` with visible warnings.

- [ ] **Step 1: Write failing quota adapter/scheduler tests**

Use a fake collector and paused Tokio time. Assert:

- Claude/Codex/Coding Plan tiers normalize into optional five-hour/seven-day windows and optional manual reset count;
- success appends a snapshot;
- failure preserves last success and waits until the next configured cycle (no tight retry);
- default interval is 300 seconds; zero disables; 1-59 is invalid;
- manual refresh invokes one collection immediately;
- metered/disabled Provider refresh is rejected.

- [ ] **Step 2: Write failing Session binding tests**

Assert unbound Claude/Codex sources import zero and return `warnings=["no usage source binding for <source>"]`. After binding, every event uses that Provider. Exact stable IDs link; token/time similarity without ID does not link. Quota snapshots remain absent from events.

- [ ] **Step 3: Implement quota adapters and scheduler**

Add an object-safe `QuotaCollector` adapter boundary. Reuse current service parsers instead of duplicating HTTP logic. A success appends snapshot and clears stale/error; a failure updates fetch state only. Add `quota_service` plus cancellable scheduler handle to `AppState`, start after DB initialization, stop during shutdown.

- [ ] **Step 4: Refactor Session parsers to emit records**

Keep existing file traversal/format parsing, but replace direct `proxy_request_logs` writes with parsed records consumed by `SessionUsageService`. Resolve `usage_source_bindings` before scanning. Update `session_log_sync` offset only after every yielded record for that file is ingested. Remove fingerprint (`DedupKey`/model+tokens+time) from the new path. First version exposes only Claude and Codex binding keys.

- [ ] **Step 5: Run suites and commit**

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage::quota -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml usage::session -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml subscription -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml coding_plan -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml session_usage -- --nocapture
git add src-tauri/src/usage src-tauri/src/store.rs src-tauri/src/lib.rs src-tauri/src/services/subscription.rs src-tauri/src/services/coding_plan.rs src-tauri/src/services/session_usage.rs src-tauri/src/services/session_usage_codex.rs
git commit -m "feat(usage): collect quotas and bound sessions"
```

### Task 6: Implement Aggregation and the Nine Tauri Commands

**Status:** Complete and independently reviewed (`fd8f7af6`, `2194c36d`).

**Files:**
- Create: `src-tauri/src/usage/dashboard.rs`
- Create: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/usage_dashboard_commands.rs`
- Test: `src-tauri/src/usage/dashboard.rs`

**Interfaces:**
- Consumes: all v13 services/DAO.
- Produces: correct product DTOs, event pagination and the exact nine public commands from the source plan.

- [x] **Step 1: Write failing aggregation tests**

Seed one product containing subscription and metered Providers, linked and unlinked cross-source events, upstream/estimated/unavailable costs, and quota snapshots. Assert:

- subscription and metered arrays remain separate;
- only linked duplicate event is excluded from sums, but remains queryable in event detail;
- similar no-ID events both count;
- quota snapshot count in Token/cost aggregation is zero;
- cost-source counts are preserved;
- half-open time range, Provider/product filters and page boundaries are exact.

- [x] **Step 2: Implement aggregation**

Exclude `usage_event_links.duplicate_event_id` only from aggregate SQL. Sum costs with `Decimal`, not SQLite REAL. Group first by Provider, then product. Read latest quota/fetch state after event aggregation and attach only to subscription cards.

- [x] **Step 3: Write failing command tests**

Through isolated AppState/test hooks cover provider save/list/enable, binding, empty and populated dashboard, page validation, manual quota refresh and Session sync warning. Serialize every command result and assert stored secrets are absent.

- [x] **Step 4: Implement and register commands**

Create and register exactly:

```text
list_usage_providers
save_usage_provider
set_usage_provider_enabled
get_route_bindings
set_route_binding
get_usage_dashboard(startAt, endAt, productGroupId?)
get_usage_events(providerId, startAt, endAt, page, pageSize)
refresh_provider_quota(providerId)
sync_provider_session_usage(providerId)
```

Commands are thin service adapters. Validate `startAt < endAt`, page size 1-200 and Provider existence before dispatch.

- [x] **Step 5: Run backend gate and commit**

```bash
cargo test --manifest-path src-tauri/Cargo.toml usage::dashboard -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test usage_dashboard_commands -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/usage/dashboard.rs src-tauri/src/usage/mod.rs src-tauri/src/commands/usage_dashboard.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/usage_dashboard_commands.rs
git commit -m "feat(usage): expose product dashboard API"
```

### Task 7: Add the Minimal React Dashboard and Configuration Surface

**Status:** Initial implementation and task-level review fixes complete (`f9063ff8`, `66cff322`, `44ae009a`). Whole-branch review reopened live time-range progression and explicit Session-source configuration; fixes are in progress.

**Files:**
- Create: `src/types/usageDashboard.ts`
- Create: `src/lib/api/usageDashboard.ts`
- Create: `src/lib/query/usageDashboard.ts`
- Modify: `src/lib/api/index.ts`
- Modify: `src/lib/query/index.ts`
- Create: `src/components/usage-dashboard/UsageDashboardPage.tsx`
- Create: `src/components/usage-dashboard/ProductUsageGroup.tsx`
- Create: `src/components/usage-dashboard/SubscriptionProviderCard.tsx`
- Create: `src/components/usage-dashboard/MeteredProviderCard.tsx`
- Create: `src/components/usage-dashboard/UsageProviderDialog.tsx`
- Create: `src/components/usage-dashboard/RouteBindingsPanel.tsx`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `src/i18n/locales/{en,zh,zh-TW,ja}.json`
- Test: `src/lib/query/usageDashboard.test.tsx`
- Test: `src/components/usage-dashboard/UsageDashboardPage.test.tsx`

**Interfaces:**
- Consumes: Task 6 wire contract and existing date range/proxy controls.
- Produces: types/API/hooks plus product group UI, separate cards, Provider/routes, proxy toggle, quota refresh and Session sync.

- [ ] **Step 1: Write failing wire/hook tests**

Mock Tauri invoke. Verify exact camelCase payloads for all nine commands and deterministic query keys containing all range/filter/page values. Provider/route mutations invalidate providers, bindings and dashboard; quota refresh invalidates that Provider/dashboard.

- [ ] **Step 2: Implement TypeScript contract and data client**

Mirror Rust types exactly:

```ts
export type BillingKind = "subscription" | "metered";
export type TokenSource = "proxy" | "session_log";
export type CostSource = "upstream" | "estimated" | "unavailable";
```

Add `usageDashboardApi`, query keys, list/dashboard/events queries and save/enable/bind/refresh/sync mutations. Do not expose a credential read API.

- [ ] **Step 3: Write failing page tests**

Mock hooks with a product containing both kinds. Verify separate subscription/metered cards, source labels, upstream/estimated/unavailable labels, stale quota while last success remains visible, today/7d/30d/custom ranges, route save, proxy start/stop, quota refresh and Session sync warning.

- [ ] **Step 4: Implement the minimal page**

Reuse existing `UsageDateRangePicker`, UI primitives and proxy hooks. Do not add complex charts. Product header shows time-scoped Token summary plus sources. Provider dialog edits v13 fields; secret inputs are blank on edit and omission preserves stored secret. Route selector lists only enabled metered Providers. Replace the old Settings `usage` tab body with the new page.

- [ ] **Step 5: Run frontend gate and commit**

```bash
pnpm test:unit -- src/lib/query/usageDashboard.test.tsx
pnpm test:unit -- src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm typecheck
pnpm build:renderer
git add src/types/usageDashboard.ts src/lib/api/usageDashboard.ts src/lib/query/usageDashboard.ts src/lib/api/index.ts src/lib/query/index.ts src/components/usage-dashboard src/components/settings/SettingsPage.tsx src/i18n/locales
git commit -m "feat(ui): add provider-aware usage dashboard"
```

### Task 8: Exit Legacy Features from the Main Path and Run End-to-End Acceptance

**Status:** In progress. Main-path UI and shell (`ee93e413`, `44ae009a`), real proxy E2E (`15e44174`, `4fa50dea`), acceptance runbook (`9575ab78`) and request-path legacy selector removal (`0e27e91c`) are complete. Whole-branch review fixes migrated-route authority/URL redaction (`bcc63b52`) and the one-upstream-attempt contract (`d0fd6f33`); Session ownership and live range fixes plus the repeat full gate remain.

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `src/components/providers/ProviderActions.tsx`
- Modify: `src/components/providers/ProviderCard.tsx`
- Modify: `src/components/proxy/ProxyPanel.tsx`
- Create: `src/App.usage-dashboard.test.tsx`
- Create: `src-tauri/tests/usage_dashboard_proxy_e2e.rs`
- Create: `docs/usage-dashboard-acceptance.md`

**Interfaces:**
- Consumes: complete v13 backend/frontend.
- Produces: hidden legacy entry points, no legacy request-path fallback, mock-upstream acceptance evidence and runbook. Old source modules/tables remain for the compatibility window.

- [ ] **Step 1: Write failing navigation visibility tests**

Render App and assert dashboard, Provider/routes and proxy controls are visible while quick switching, failover, preset marketplace, MCP, Skills, OpenClaw, WebDAV and S3 entry points are absent. Old settings tab names must fall back to the usage page, not a blank panel.

- [ ] **Step 2: Hide legacy UI and keep compatibility code compiled**

Remove the legacy buttons/actions from the rendered tree. Do not delete old Tauri modules, source files, database tables, migrations or settings keys. Keep old log dual-write and backup/export compatibility.

- [ ] **Step 3: Write and pass a real mock-upstream test**

Start an Axum upstream on port 0. Seed metered Provider plus static binding, start `ProxyServer` on port 0, send one real request containing explicit Token/cost response, wait for ingestion, and query dashboard/events. Assert exact Provider/time/tokens/cost and `costSource=upstream`. Add response without cost (`estimated`) and route-less request (local 503, zero upstream hits).

- [ ] **Step 4: Document manual acceptance and run the full gate**

`docs/usage-dashboard-acceptance.md` must contain safe fixture config, expected 503 bodies, expected event fields, SQLite read-only queries and explicit checks that quota never enters events/trends and no-ID similar events remain separate.

Run:

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
pnpm typecheck
pnpm test:unit
pnpm build:renderer
rg -n "get_effective_current_provider|get_failover_queue|select_providers" src-tauri/src/proxy/provider_router.rs src-tauri/src/proxy/handler_context.rs src-tauri/src/proxy/forwarder.rs
git diff --check
```

Expected: all tests/builds pass; request-path `rg` has no match; legacy compatibility files still exist but are unreachable from the new path/UI. Cargo 输出必须来自仓库固定的 Rust 1.95 toolchain，不能用此前其他提交的成功结果代替。

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/components/settings/SettingsPage.tsx src/components/providers/ProviderActions.tsx src/components/providers/ProviderCard.tsx src/components/proxy/ProxyPanel.tsx src/App.usage-dashboard.test.tsx src-tauri/tests/usage_dashboard_proxy_e2e.rs docs/usage-dashboard-acceptance.md
git commit -m "test(usage): complete dashboard acceptance path"
```

## Coverage Map

- v13 migration, old data retention, idempotency and rollback: Task 1.
- Provider billing/token model, redaction, static/source bindings, immutable events and quota persistence: Tasks 1-2.
- One fixed route, next-request activation, local 503 and no fallback: Task 3.
- Upstream/estimated/unavailable costs, decimal/null semantics, atomic dual-write, diagnostic-only logging failure and exact stable-ID linking: Task 4.
- Five-minute quota schedule/manual refresh, last-success preservation and explicit Session ownership: Task 5.
- Product grouping, separate subscription/metered DTOs, quota exclusion, filters and pagination, nine commands: Task 6.
- Basic frontend display/config/routes/proxy/refresh/sync and source/stale labels: Task 7.
- Milestone 3 main-path exit/hidden entrances, full test gate and real proxy acceptance: Task 8.
