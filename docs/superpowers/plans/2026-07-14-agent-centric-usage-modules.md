# Agent-Centric Usage Modules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` task-by-task and
> `superpowers:verification-before-completion` before any completion claim.

**Goal:** Replace subscription modules plus the global API module with Agent-centric
modules. Each Agent page shows that Agent's subscription and metered Providers,
and each Agent–Provider binding owns a distinct protected API credential used for
local-proxy routing and upstream authentication.

**Architecture:** Introduce additive schema v16 tables `agent_modules` and
`agent_provider_bindings`, leaving v15 display tables frozen as legacy data during
this release. New usage events freeze nullable `agent_module_id` alongside the
existing Provider identity; only new trusted imports and proxy requests receive a
non-null Agent, while ambiguous historical rows remain unassigned. The proxy
resolves an inbound credential fingerprint to exactly one enabled binding in one
database query, loads the raw credential from a protected OS store, freezes the
Agent/Provider pair in request context, and rejects every unresolved request before
network I/O. Dashboard queries use frozen event ownership, while quota remains
Provider-scoped and is labeled shared rather than duplicated.

**Tech Stack:** Rust 2021, rusqlite/SQLite, Tauri 2, Axum/Hyper, React 19,
TypeScript, TanStack Query, Vitest, pnpm, OS credential store.

## Superseded Plans

This plan supersedes the product and execution assumptions in:

- `docs/superpowers/specs/2026-07-12-subscription-dashboard-navigation-design.md`
- `docs/superpowers/plans/2026-07-13-dynamic-dashboard-frontend.md`
- `docs/superpowers/plans/2026-07-13-dashboard-module-backend.md`

Those documents remain historical records. In particular, their global `api`
module, single `usage_providers.dashboard_module_id` membership, and
subscription/API page split are no longer runtime requirements.

## Non-Negotiable Invariants

- Seed exactly five immutable fixed Agent IDs: `codex`, `claude-code`, `opencode`,
  `openclaw`, and `hermes`. Fixed Agents may be reordered or hidden but never
  renamed, deleted, or converted. Custom Agents may be created, renamed, hidden,
  reordered, archived, and deleted only when history safety permits.
- There is no global API Agent/module and no API-module creation path.
- Provider membership is many-to-many through Agent–Provider bindings. Provider
  save/update never implicitly removes bindings.
- Keep the five existing dashboard-module Tauri command names as compatibility
  entry points, but change their active semantics and DTOs to Agent modules. Saving
  a `kind`, creating `api`, or mutating a fixed identity is rejected. Frontend
  wrappers/components may use Agent-centric names.
- A binding key is both the Agent's local-proxy selection credential and the
  upstream API credential for that binding. SQLite stores only a full,
  domain-separated SHA-256 fingerprint; the raw key lives only in the protected
  credential store and transient request memory.
- No command output, DTO, event, warning, log, error, or diagnostic may contain raw
  keys, fingerprints, masked prefixes/suffixes, or derived hints. The sole command
  boundary allowed to receive a raw key is the write-only input of set/replace
  credential mutations; it deserializes directly into `SecretString` and is excluded
  from command tracing and Debug. A raw binding key introduced through the new
  credential service may not enter SQL export, binary backup, or sync artifacts.
  The full fingerprint
  is intentionally stored in the private SQLite binding row and therefore may be
  present in controlled database backup/sync; restored bindings remain effectively
  disabled until the local protected store reconciles successfully.
- A non-null fingerprint maps to at most one binding. An enabled proxy binding must
  have a locally available credential. Set/replace/clear/delete are explicit and
  transactionally ordered to fail closed.
- `effective_enabled` is true only when the binding and Provider are enabled, the
  Agent is not archived, its route uses supported direct API-key authentication,
  and the protected credential verifies. Every proxy lookup enforces the whole
  predicate in its atomic join; a merely requested `enabled=true` is insufficient.
- Inbound credentials are accepted according to an endpoint-specific extraction
  matrix, must agree when the same credential role is repeated, and are removed
  before URL construction, logging, or upstream forwarding. For ordinary Claude,
  Codex, and Gemini routes the binding key may use Bearer, the protocol-native key
  header, or sanitized query `key`; for `/claude-desktop/*`, Authorization Bearer
  is exclusively the gateway token and the binding key must use `x-api-key`.
  Unknown, missing, disabled, store-missing, or protocol-mismatched credentials
  return one generic local authorization failure and produce zero upstream requests
  and zero fallback attempts.
- Binding resolution is the authorization linearization point and runs immediately
  before an upstream request is constructed. A binding disabled or rotated before
  resolution is rejected; an already-resolved in-flight request retains its frozen
  route and credential. Never hold the SQLite mutex or credential-store lock across
  network I/O.
- `RequestContext` freezes `agent_module_id`, `provider_id`, and binding identity
  once. Response processing and asynchronous ingestion never re-resolve current
  bindings. Binding edits are prospective and cannot move history.
- New event ingestion requires an explicit trusted Agent. Migrated ambiguous
  historical rows retain `agent_module_id = NULL`; they are visible only through
  unassigned diagnostics and excluded from Agent totals.
- Stable cross-source matching and links require the same non-null Agent and
  Provider. Proxy/upstream cost remains canonical regardless of arrival order.
- Claude session parsing always attributes to `claude-code`; Codex session parsing
  always attributes to `codex`. Neither parser infers Agent from current UI state.
- Quota snapshots/fetch state remain Provider-owned. The same Provider card may
  appear under multiple Agents with a shared-account label, but quota is fetched
  once and never summed into Agent usage totals.
- The application reports setup information but never edits Agent configuration.
- Do not launch the desktop app against the user's real home or application data.
- Run Rust only through `pnpm rust -- ...`; run Tauri through pnpm wrappers.

## Credential Lifecycle Contract

The current repository has no general protected secret writer. This plan therefore
adds a `CredentialStore` abstraction rather than storing new keys in
`usage_providers.route_config` or SQLite backups.

- The current release target uses
  `security-framework = "3.7"` and
  `security_framework::passwords::{set_generic_password,
  get_generic_password, delete_generic_password}` behind a macOS adapter. Linux CI
  and any non-release target compile an `UnavailableCredentialStore` that fails
  closed; adding a native Windows/Linux production adapter is required before that
  target may ship. Tests inject an in-memory store and never touch an OS store.
  `zeroize = "1"` backs the non-serializable secret input wrapper.
- Service: `com.xr810.llm-usage-bar.agent-provider-binding.v1`.
- Account/slot: a fresh opaque UUID for every credential generation; never the key,
  fingerprint, stable binding ID, or an in-place-updated slot. The private binding
  row points to the active slot, while the journal maps pending/old slots.
- Each mutation uses a versioned staging slot and a non-secret
  `agent_credential_operations` journal. A partial unique pending-operation index
  plus `BEGIN IMMEDIATE` serializes one binding even across processes; different
  bindings remain independent. The DB compare-and-swap commit is the linearization
  point.
- `set`: require version 0/no active fingerprint, reserve generation 1 and staging
  slot in the journal, write protected item, then CAS the binding to the new
  fingerprint/slot/version. On CAS failure, delete only that staging slot.
- `replace`: reserve `current_version + 1`, write a new immutable slot, CAS the DB
  pointer, then mark cleanup and delete the old slot. A failure can never overwrite
  or delete another successful generation.
- `clear`: reserve the next generation, CAS the DB pointer/fingerprint to NULL and
  effective-disabled, then delete the formerly active slot. A deletion failure
  leaves an unreachable journaled orphan and never restores routing.
- `delete`: prohibit FK cascade, clear through the service, then remove the binding;
  Provider/Agent deletion uses `RESTRICT` until this lifecycle succeeds.
- Startup reconciliation completes/cleans journaled operations, marks missing or
  mismatched active slots unavailable/effectively disabled, and never guesses a
  secret. Every lookup still validates DB fingerprint against the protected value.
- Lookup fingerprints the inbound key, loads one enabled binding atomically, reads
  its protected item, and compares fingerprints in constant time before returning
  an opaque resolved route. Store absence or mismatch fails closed.
- Do not auto-adopt or scrub credentials from legacy `usage_providers.route_config`,
  `providers.settings_config`, `proxy_live_backup`, or `quota_config`. Those values
  are owned by pre-existing Provider switching/quota features, and rewriting them
  would expand this task and can break unrelated behavior. The new request path
  ignores legacy route credentials entirely and uses only a verified binding
  credential. Migrated proxy bindings therefore remain disabled/missing until the
  user explicitly sets a key through the binding UI. Only keys entered through that
  new boundary are covered by this plan's protected-store, no-export, and no-leak
  guarantees; legacy credential storage remains documented pre-existing debt.

## Migration Policy

- Add schema v16 without renaming or rebuilding v15 tables. Keep
  `dashboard_modules` and `usage_providers.dashboard_module_id` as legacy-only
  columns so dependent foreign keys and user data remain intact.
- Create `agent_modules` with soft-archive support, fixed/custom constraints, and
  mutation triggers; create `agent_provider_bindings` with a stable UUID primary
  key, unique `(agent_module_id, provider_id)`, unique non-null fingerprint,
  private `credential_slot`, monotonic `credential_version`, requested enabled
  state, and timestamps. Create the non-secret credential-operation journal used by
  Task 3; no raw secret is ever stored there.
- Add nullable `usage_events.agent_module_id` plus lookup indexes. Update the
  immutable-event trigger to include the new column. The migration must drop the
  old immutable UPDATE trigger inside its savepoint, perform only proven attribution
  backfill, and recreate the trigger before validation; rollback must restore the
  original trigger and rows.
- Seed five fixed Agents. Preserve visibility/order for legacy Codex and Claude
  rows where possible. Copy user-created custom legacy modules. Preserve a used
  `kimi-coding-plan` as Custom and leave its Providers unassigned for review;
  omit an unused default Kimi row. Never migrate `api` or
  `other-subscriptions` into an Agent.
- Backfill current display bindings only from exact, unambiguous user configuration:
  `usage_source_bindings.source='codex'` may create a disabled Codex subscription
  binding and `source='claude'` a disabled Claude Code subscription binding. A
  membership in a truly user-created custom v15 module may be copied disabled.
  Legacy default Codex/Claude/Kimi memberships are not evidence because v15 derived
  them from vendor labels; those Providers remain unassigned unless the independent
  source-binding evidence exists. No migration creates an enabled proxy binding or
  interprets protocol compatibility as Agent identity.
- Backfill historical `agent_module_id` only when immutable evidence is exact, in
  this precedence order:
  1. A `legacy_request_id` join with `data_source='session'`,
     `app_type='claude'`, `provider_type='session_log'` maps to `claude-code`; the
     analogous `app_type='codex'`, `provider_type='codex_session'` maps to `codex`.
     Compatibility values `session_log` and `codex_session` are accepted only with
     the same matching app/provider-type pair.
  2. A session event generated with immutable ID prefix `claude-session:` maps to
     `claude-code`; `codex-session:` maps to `codex`.
  3. A `legacy_request_id` join whose `data_source='proxy'` maps normalized
     `app_type` exactly as `codex→codex`, `claude→claude-code`,
     `opencode→opencode`, `openclaw→openclaw`, or `hermes→hermes`.
  Bare `source='session_log'`, product group, vendor label, or current Provider state
  is never enough. Conflicting evidence leaves the event/component NULL. Labels
  `openai`, `anthropic`, `moonshot`, `kimi`, and `api` are explicitly unproven.
- Before historical backfill, audit every existing `usage_event_links` connected
  component. Attribute a component only when every proven candidate agrees on one
  Agent; conflicting or one-sided evidence keeps the uncertain rows NULL. Preserve
  link rows, but runtime duplicate exclusion accepts a link only when both endpoints
  share the same non-null Agent and Provider. Invalid legacy links are diagnostics.
- Migration tests compare event/provider/quota/link counts before and after and
  prove unknown history is neither guessed, duplicated, nor discarded.
- Existing source bindings continue selecting a Provider only. Claude/Codex parsers
  supply their fixed Agent ID and require the corresponding Agent–Provider binding;
  no mutable source setting is allowed to override that fixed identity.
- Custom deletion semantics are exact: a Custom Agent with any historical event is
  soft-archived; one with no history and no binding may be hard-deleted. Archived
  Agents are absent from navigation and new binding choices, remain resolvable for
  historical labels, and are included in archived/unassigned diagnostics. Archival
  immediately makes every binding ineffective in the same DB transaction, then the
  credential lifecycle clears/deletes its bindings; cleanup failure can leave only
  an unreachable protected orphan. Tests prove archive yields zero upstream hits
  and cannot create a new event even before store cleanup finishes.

---

### Task 0: Persist and review this execution plan

**Files:**

- Create: `docs/superpowers/plans/2026-07-14-agent-centric-usage-modules.md`
- Create: `.superpowers/sdd/progress.md`

- [x] Confirm the isolated worktree is clean and based on `e653d977`.
- [x] Review this plan against the restored user specification and the three code
  maps: persistence, proxy/ingestion, and frontend.
- [x] Search for placeholders, conflicting global API assumptions, and missing
  verification commands.
- [x] Commit the plan before production code.

Verification:

```bash
rg -n "TODO|TBD|dashboardModuleId|global API" \
  docs/superpowers/plans/2026-07-14-agent-centric-usage-modules.md
git diff --check
```

Commit: `docs: plan agent-centric usage modules`

### Task 1: Add schema v16 and conservative history attribution

**Files:**

- Create: `src-tauri/src/usage/agent_module_migration.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/tests.rs`
- Modify: `src-tauri/src/usage/migration.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: pre-existing integration fixtures missing `dashboard_module_id`

**Interfaces:** schema version 16; tables `agent_modules` and
`agent_provider_bindings`; nullable `usage_events.agent_module_id`.

- [ ] RED: add v15→v16 tests for exactly five fixed Agents/no API, custom/Kimi
  preservation rules, binding backfill, exact history mapping, unknown NULL rows,
  no row-count loss, trigger immutability, continuous v12→v16 upgrade, and full
  rollback after an injected failure.
- [ ] History fixtures include exact Claude/Codex session markers and ID prefixes,
  exact proxy app types, bare session/product/vendor labels that must stay NULL, and
  a conflicting linked component that must stay wholly NULL.
- [ ] Expected RED before implementation: the focused test either cannot resolve
  `agent_module_migration` or reports `left: 15, right: 16` / `no such table:
  agent_modules`. A test that passes before production code is strengthened until it
  proves the missing behavior.
- [ ] Run the focused migration tests and capture the expected missing-schema
  failure.
- [ ] GREEN: implement the additive migration and v16 completeness validator.
  Do not write the credential store or runtime DAO in this task.
- [ ] Preserve the DELETE trigger, then inside the schema savepoint drop the full
  UPDATE trigger, add the nullable column, audit link components, perform exact
  backfill, recreate the full UPDATE trigger, verify the trigger rejects an UPDATE,
  and only then set `user_version=16`. Inject failures after trigger drop, during
  backfill, and before trigger recreation to prove complete rollback.
- [ ] Repair the three known integration fixture compilation failures only by
  adding their missing legacy field; do not change their assertions yet.
- [ ] Run migration/schema tests, Rust formatting, and the full Rust library suite.
- [ ] Request spec-compliance review, then code-quality review; resolve findings.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::tests::migration_v15_to_v16 --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::tests::schema_migration --lib -- --nocapture
pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(usage): migrate agent modules and event ownership`

### Task 2: Implement Agent module and binding persistence

**Files:**

- Create: `src-tauri/src/database/dao/agent_modules.rs`
- Create: `src-tauri/src/database/dao/agent_provider_bindings.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Replace active implementation in: `src-tauri/src/database/dao/dashboard_modules.rs`
- Modify: `src-tauri/src/database/dao/usage_providers.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: Rust fixtures constructing Provider/module DTOs

**Interfaces:**

```rust
pub struct AgentModuleInput {
    pub id: Option<String>, pub name: String, pub sort_order: i64, pub visible: bool,
}
pub struct AgentModuleView {
    pub id: String, pub name: String, pub sort_order: i64, pub visible: bool,
    pub is_fixed: bool, pub archived_at: Option<i64>, pub provider_count: u64,
}
pub struct AgentProviderBindingInput {
    pub id: Option<String>, pub agent_module_id: String,
    pub provider_id: String, pub enabled: bool,
}
pub enum BindingCredentialStatus { NotRequired, Missing, Configured, Unavailable }
pub struct AgentProviderBindingView {
    pub id: String, pub agent_module_id: String, pub provider_id: String,
    pub enabled: bool, pub effective_enabled: bool,
    pub credential_status: BindingCredentialStatus,
    pub credential_version: u64, pub created_at: i64, pub updated_at: i64,
}
```

- [ ] RED: tests for five fixed rows; fixed rename/delete rejection; fixed/custom
  reorder/hide; Custom create/rename/archive; history-safe delete; many-to-many
  membership; unique pair/fingerprint constraints; provider deletion protections;
  Provider save preserving bindings; and public DTO serialization without secret
  fields.
- [ ] Expected RED before implementation: unresolved `AgentModuleInput` /
  `AgentProviderBindingView` or missing `Database::list_agent_modules`; no test may
  be accepted as RED merely because an unrelated fixture does not compile.
- [ ] GREEN: implement Agent module CRUD and binding metadata CRUD in one
  transaction per mutation. Runtime stops reading v15 module membership.
- [ ] Until Task 3 exists, proxy-capable/API-key bindings can only be created
  disabled. Session-only subscription bindings use `NotRequired` and may be enabled.
  OAuth/Copilot/managed-auth Providers are display/session bindings only and reject
  binding credential operations with public code `unsupported_auth`; an API key
  never overrides their managed authentication.
- [ ] Keep `list/save/reorder/set_visibility/delete_dashboard_module` command names
  wired to the new DAO. `save_dashboard_module` accepts only the Agent input above;
  legacy payloads containing `kind` or ID `api` fail with `invalid_agent_module`.
- [ ] Make `UsageProviderView.bindings` authoritative and remove projected
  `dashboard_module_id` from the active DTO while leaving the legacy DB column.
- [ ] Run focused DAO/domain tests, format, and full library tests.
- [ ] Complete two-stage review and resolve all findings.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::dao::agent_modules::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::dao::agent_provider_bindings::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::dao::usage_providers::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(usage): persist agent provider bindings`

### Task 3: Add protected binding credentials and atomic lifecycle

**Files:**

- Create: `src-tauri/src/credentials/mod.rs`
- Create: platform adapter(s) selected by `cfg`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/database/dao/agent_provider_bindings.rs`

**Interfaces:** `CredentialStore` trait; `BindingCredentialService`;
`set/replace/clear` operations; opaque `ResolvedBindingCredential`.

```rust
pub trait CredentialStore: Send + Sync {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError>;
    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError>;
    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError>;
}

/// Non-Clone, non-Serialize input whose Debug implementation always prints
/// `SecretString([REDACTED])` and whose inner bytes are zeroized on drop.
pub struct SecretString(/* private */ String);

pub async fn set_binding_api_key(
    binding_id: &str, expected_version: u64, api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError>;
pub async fn replace_binding_api_key(
    binding_id: &str, expected_version: u64, api_key: SecretString,
) -> Result<AgentProviderBindingView, AppError>;
pub async fn clear_binding_api_key(
    binding_id: &str, expected_version: u64,
) -> Result<AgentProviderBindingView, AppError>;
```

`AppState` owns `Arc<dyn CredentialStore>` and `Arc<BindingCredentialService>`.
`AppState::new_with_credential_store` and `ProxyService::new_with_credential_store`
are dependency-injection constructors used by tests; production creates the macOS
adapter before AppState/proxy construction. Store calls are synchronous APIs and
are always run in `tokio::task::spawn_blocking`, never while holding a SQLite
mutex. Public errors are
limited to `binding_not_found`, `invalid_binding`, `credential_required`,
`credential_conflict`, `credential_unavailable`, and `unsupported_auth`; underlying
store/database strings are logged only as generic operation failures.

- [ ] RED: in-memory-store tests for set-vs-replace rules, rollback/restore after DB
  failure, clear/delete orphan-safe ordering, duplicate key rejection, disabled
  binding rejection, store-missing/mismatch status, and concurrent rotation.
- [ ] Expected RED before implementation: missing `CredentialStore` or credential
  operations, followed by behavior failures such as a second `set` not returning
  `credential_conflict`. Record the exact focused failure in the task ledger.
- [ ] RED: serialization/log/export tests assert neither newly entered binding keys
  nor fingerprints/prefixes/suffixes appear in public surfaces. SQL/binary
  backup/sync may contain the private binding fingerprint but must never contain the
  newly entered raw binding key.
- [ ] GREEN: implement the OS-protected production adapter and dependency with the
  smallest supported feature set. CI/test builds inject memory storage and never
  touch the user's credential store.
- [ ] Use full domain-separated SHA-256 and constant-time equality. Never log
  credential-store error payloads that could contain secret metadata.
- [ ] Run focused credential/backup tests, full library tests, and dependency diff
  review.
- [ ] Complete security-oriented spec and quality reviews.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml credentials --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(proxy): protect agent binding credentials`

### Task 4: Route proxy requests by binding key and freeze ownership

**Files:**

- Modify: `src-tauri/src/proxy/server.rs`
- Modify: `src-tauri/src/proxy/handler_context.rs`
- Modify: `src-tauri/src/proxy/provider_router.rs`
- Modify: `src-tauri/src/proxy/handlers.rs`
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Modify: `src-tauri/src/proxy/error.rs`
- Modify: `src-tauri/src/proxy/error_mapper.rs`
- Modify: `src-tauri/src/proxy/response_processor.rs`
- Modify: `src-tauri/src/proxy/usage/logger.rs`
- Modify: `src-tauri/src/usage/ingestion.rs`
- Modify: `src-tauri/src/database/dao/usage_events.rs`
- Modify: `src-tauri/src/database/dao/usage_providers.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: proxy/ingestion tests and `src-tauri/tests/usage_dashboard_proxy_e2e.rs`

- [ ] RED: extraction tests cover the endpoint matrix: ordinary Claude/Codex
  Bearer and protocol-native headers; Gemini `x-goog-api-key`/sanitized query `key`;
  and Claude Desktop gateway Bearer plus a distinct `x-api-key` binding credential.
  Repeated values for one role must agree, cross-role tokens are not compared, and
  every credential location is stripped before logging/forwarding. Include
  `/claude-desktop/v1/models`; it must resolve a binding and never use static route
  selection.
- [ ] RED: real mock-upstream tests prove two keys for the same Provider isolate
  Agents, rotation takes effect at the defined pre-send lookup boundary, and
  missing/unknown/disabled/store-missing/protocol-mismatch produce generic local
  failure with an upstream hit count of exactly zero.
- [ ] Include archived-Agent routing: archiving after a successful request must make
  the same key return locally, keep upstream hits unchanged, and insert no new event
  before or after protected-slot cleanup.
- [ ] Expected RED against the static-route baseline: an unknown credential still
  reaches the configured upstream (`hits == 1`) or the resolved event lacks an Agent;
  the strengthened test must fail on that exact assertion.
- [ ] RED: event tests require frozen Agent/Provider ownership, preserve history
  after rebinding, reject duplicate event IDs with conflicting ownership, and
  ensure errors/DTO/log rows contain no credential material.
- [ ] GREEN: replace request-path static route selection with one atomic binding
  resolution. Preserve legacy route-binding commands only as dormant compatibility
  interfaces; no request fallback may consult them.
- [ ] Treat the binding lookup immediately before request construction as the
  linearization point. The resolved opaque value owns the protected upstream key;
  the raw inbound header/query value is discarded after comparison and is never
  used directly for forwarding.
- [ ] Strip local credentials before constructing upstream URI/headers. Override
  Provider route authentication with the protected binding key while preserving
  route base URL, protocol, model mapping, and pricing configuration.
- [ ] Build a credential-free runtime route projection before injecting that key:
  strip direct auth/key fields, nested env/auth/config/TOML auth, URL userinfo, and
  credential query parameters from legacy `route_config`. A sentinel test keeps an
  old secret in SQLite, proves the upstream URI/headers contain only the new binding
  key, and proves the existing Provider-switch read/write path remains unchanged.
- [ ] Thread frozen IDs through streaming, non-streaming, logging, and asynchronous
  ingestion. Do not re-check mutable binding state after a response is sent.
- [ ] Run protocol-focused tests, proxy integration test outside sandbox, full
  library suite, and two-stage review.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml proxy:: --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::ingestion --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  --test usage_dashboard_proxy_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(proxy): route by agent binding credential`

### Task 5: Make session ingestion and dedup Agent-safe

**Files:**

- Modify: `src-tauri/src/usage/session.rs`
- Modify: `src-tauri/src/services/session_usage.rs`
- Modify: `src-tauri/src/services/session_usage_codex.rs`
- Modify: `src-tauri/src/usage/ingestion.rs`
- Modify: `src-tauri/src/database/dao/usage_events.rs`
- Modify: `src-tauri/src/usage/domain.rs`

- [ ] RED: Claude and Codex parser tests prove fixed Agent identity regardless of
  current Provider bindings; wrong source/Agent bindings reject locally; rebinding
  and rescan cannot move prior events; Codex files without session IDs cannot
  collide globally.
- [ ] RED: cross-source matching/link tests require identical non-null
  `(agent_module_id, provider_id)` and prove proxy/upstream-cost canonical ownership
  for both session-first and proxy-first arrival orders.
- [ ] Expected RED against the provider-only baseline: different-Agent events link
  or a parsed event has no fixed Agent. Capture the first behavior assertion rather
  than unrelated compilation fallout.
- [ ] GREEN: carry trusted Agent constants through session inputs and DAO queries;
  make canonical selection deterministic without mutating immutable events.
- [ ] Run all session/ingestion/event DAO tests and the full library suite.
- [ ] Complete two-stage review and resolve findings.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  services::session_usage --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  services::session_usage_codex --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  database::dao::usage_events::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(usage): freeze trusted agent attribution`

### Task 6: Expose Agent dashboard, events, diagnostics, and commands

**Files:**

- Modify: `src-tauri/src/usage/dashboard.rs`
- Modify: `src-tauri/src/database/dao/usage_events.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: `src-tauri/src/usage/quota.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/usage_events.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/usage_dashboard_commands.rs`

**Interfaces:** Agent-filtered `UsageDashboardView`; Agent-filtered event page;
binding CRUD and key lifecycle commands; proxy setup info; unassigned diagnostics;
secret-free global invalidation event.

Tauri command contract:

```text
list_dashboard_modules() -> AgentModuleView[]
save_dashboard_module(input: AgentModuleInput) -> AgentModuleView
reorder_dashboard_modules(moduleIds: string[]) -> AgentModuleView[]
set_dashboard_module_visibility(moduleId: string, visible: bool) -> AgentModuleView
delete_dashboard_module(moduleId: string) -> void
list_agent_provider_bindings(agentModuleId?: string) -> AgentProviderBindingView[]
save_agent_provider_binding(input: AgentProviderBindingInput) -> AgentProviderBindingView
delete_agent_provider_binding(bindingId: string, expectedVersion: u64) -> void
set_agent_provider_binding_api_key(bindingId: string, expectedVersion: u64, apiKey: string)
replace_agent_provider_binding_api_key(bindingId: string, expectedVersion: u64, apiKey: string)
clear_agent_provider_binding_api_key(bindingId: string, expectedVersion: u64)
get_agent_proxy_setup_info(agentModuleId: string) -> AgentProxySetupInfo
get_unassigned_usage_diagnostics() -> UnassignedUsageDiagnostics
get_usage_dashboard(startAt: i64, endAt: i64, agentModuleId: string)
get_usage_events(agentModuleId: string, providerId?: string, startAt: i64,
                 endAt: i64, page: u64, pageSize: u64)
```

The three key commands deserialize directly into `SecretString`; test/debug output
must be redacted. `get_route_bindings`/`set_route_binding` remain registered for
legacy Settings compatibility but never participate in request routing.

- [ ] RED: dashboard tests combine subscription and metered cards for one Agent,
  exclude other/unassigned events, retain historical providers after unbinding,
  label Provider quota shared without summing it, and avoid duplicate aggregation
  for many-to-many joins.
- [ ] RED: event command tests require `agent_module_id`; diagnostics report NULL
  history by Provider/source/count/range without guessing; every public JSON
  snapshot is credential-free.
- [ ] RED: command hooks cover fixed/custom module semantics, binding CRUD,
  set/replace/clear, proxy setup, diagnostics, and command registration.
- [ ] Expected RED: the old dashboard command accepts only `product_group_id` or
  returns cross-Agent Provider totals; registration-source checks name every new
  handler exactly once.
- [ ] GREEN: query Agent history from immutable event ownership; provider cards are
  the union of active bindings and in-range historical events. Quota remains one
  Provider snapshot and `shared_account` is derived from current active bindings.
- [ ] Derive `shared_account` as more than one distinct effective binding to a
  non-archived Agent. Manual refresh and scheduler collection use a Provider-ID
  keyed singleflight so concurrent refreshes from two Agent pages share one fetch
  result/snapshot; dashboard aggregation obtains distinct Provider IDs before
  attaching quota outside the usage-event sum.
- [ ] Emit a payload-free `usage-dashboard-invalidated` event after binding/key
  mutations and usage writes.
- [ ] Run dashboard/command/integration tests, format, and full library suite.
- [ ] Complete two-stage review and resolve findings.

Verification:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  usage::dashboard --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  commands::usage_dashboard::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml \
  --test usage_dashboard_commands -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
```

Commit: `feat(usage): query agent dashboards and diagnostics`

### Task 7: Replace global API UI with unified Agent pages and Settings

**Files:**

- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/usageDashboard.ts`
- Modify: `src/lib/query/usageDashboard.ts`
- Modify: `src/hooks/useUsageEventBridge.ts`
- Modify: `src/App.tsx`
- Modify/rename: `src/components/usage-dashboard/DashboardModuleSwitcher.tsx`
- Modify/rename: `src/components/usage-dashboard/useDashboardModuleSelection.ts`
- Create: `src/components/usage-dashboard/AgentUsagePage.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.tsx`
- Modify: `src/components/usage-dashboard/usageDashboardProjection.ts`
- Remove/replace: global `ApiUsagePage` and old module page split
- Modify/rename: `src/components/settings/DashboardModulesSettings.tsx`
- Create: binding row, protected-key dialog, setup-info, and diagnostics components
- Modify: `src/components/settings/UsageProvidersSettings.tsx`
- Modify: `src/components/usage-dashboard/UsageProviderDialog.tsx`
- Modify: `src/components/settings/ProxyRoutingSettings.tsx`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `src/i18n/locales/{en,zh,zh-TW,ja}.json`
- Modify: frontend unit/integration fixtures and `tests/msw/handlers.ts`

- [ ] RED: types/API/query tests require Agent IDs in dashboard/event cache keys,
  binding/key/setup/diagnostic APIs, and exhaustive invalidation after mutations.
- [ ] RED: navigation tests render five fixed Agents plus Custom, allow fixed
  reorder/hide, reject fixed rename/delete, preserve selection/fallback, and prove
  no API navigation or route/proxy mutation on tab change.
- [ ] RED: projection/page tests show subscription and metered Provider sections
  together, calculate only Agent-scoped metered totals, deduplicate shared Provider
  cards, show shared-account quota labels, and never display another Agent's prior
  cache data.
- [ ] RED: Settings tests cover explicit Custom creation; binding add/enable/delete;
  key Set/Replace/Clear via password-only transient inputs; safe proxy instructions;
  unassigned diagnostics; Provider edit preserving bindings; and no Agent config
  file writes.
- [ ] Expected RED: TypeScript reports missing Agent/binding DTO fields and the
  existing App test still renders the `API` tab. Focused UI tests must demonstrate
  that old global page before implementation removes it.
- [ ] GREEN: implement the smallest unified Agent UI. Reuse Provider cards and
  time-range/session/quota controls; remove the global API page branch and legacy
  module selector from Provider editing.
- [ ] Update all four locales and MSW/integration fixtures. Public UI may show only
  credential `Configured`, `Missing`, or `Unavailable`; never a masked key.
- [ ] Run focused tests after each slice, then full frontend tests, typecheck,
  renderer build, and formatting.
- [ ] Complete two-stage review and resolve findings.

Verification:

```bash
pnpm test:unit src/lib/query/usageDashboard.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm test:unit src/components/settings/DashboardModulesSettings.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
pnpm test:unit
pnpm typecheck
pnpm build:renderer
pnpm format:check
```

Commit: `feat(ui): organize usage by agent`

### Task 8: End-to-end security and regression acceptance

**Files:**

- Modify as required by test findings only.
- Create/update: isolated acceptance notes under `.superpowers/sdd/`.

- [ ] Add real local-upstream acceptance for Claude, Codex, and Gemini protocols.
  For each, configure two binding keys where applicable, verify frozen Agent
  ownership, and assert no credential appears in captured upstream URI, app logs,
  request logs, command output JSON, diagnostics, or frontend snapshots. Live DB
  dump/export/sync/backup must contain no raw binding key introduced by the new
  credential service; a fingerprint is permitted only in private binding/index
  columns, and explicitly out-of-scope legacy Provider credentials may remain.
- [ ] Prove unknown/disabled/cleared keys return locally with upstream hit count 0.
- [ ] Prove rebind/rotate/delete operations do not alter historical event rows.
- [ ] Run all focused suites, then the complete verification matrix outside the
  sandbox where local listeners require it.
- [ ] Review every commit and the aggregate diff against this plan. Run
  `git diff --check`, inspect untracked files, and confirm the root checkout's
  unrelated Settings close-control edits were never copied or committed.
- [ ] Request final code review. Resolve findings and rerun every affected gate.
- [ ] Use `superpowers:finishing-a-development-branch` to present integration
  options. Do not merge, push, or open a PR without the user's explicit choice.

Final verification matrix:

```bash
pnpm test:unit
pnpm typecheck
pnpm build:renderer
pnpm format:check
pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --tests
git diff --check
git diff --check e653d977...HEAD
git diff --stat e653d977...HEAD
git log --oneline e653d977..HEAD
git status --short
```

Expected: all commands exit zero. Listener-dependent Rust tests run with approved
unsandboxed execution. Any remaining warning must be explicitly categorized as a
known pre-existing baseline item with evidence; no new failure may be waived.

Commit: `test(usage): verify agent-centric routing isolation`
