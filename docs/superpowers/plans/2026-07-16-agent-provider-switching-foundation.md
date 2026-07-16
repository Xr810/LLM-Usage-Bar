# Agent Provider Switching Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish physical schema v19 and the fail-closed backend primitives that make per-Agent current selection, account provenance, dynamic proxy audiences, protected transactions, recovery, sync review, and guarded mutation possible.

**Architecture:** SQLite holds immutable logical identity plus installation-local switch metadata; protected storage holds secret generations and per-transaction recovery keys; descriptor-safe target I/O and a monotonic journal mediate every live mutation. The proxy and ingestion layers freeze Agent/Provider/account provenance at the authorization boundary, while restore and sync stage any mutation that could invalidate a locally current or uncertain route.

**Tech Stack:** Rust 2021, rusqlite/SQLite, UUIDv4/v5, HMAC-SHA256, XChaCha20-Poly1305, macOS protected credentials, Axum/Hyper, tempfile, serial_test.

**Depends on:** [`2026-07-16-agent-provider-switching.md`](2026-07-16-agent-provider-switching.md) Execution Preflight.

---

## Foundation Invariants

- Physical v18 already contains `usage_providers.daily_budget_usd`; v19 preserves it byte-for-byte.
- Migration never reads or writes an Agent configuration file and never starts the proxy.
- Existing usage/quota evidence keeps null account fields and is rendered as `legacy_unscoped`; migration never guesses historical claims.
- Every Agent receives one permanent `agent_switch_states` row. Custom Agents start `setup_required`; built-ins start non-current and await read-only detection.
- `current_binding_id` is valid only with `config_state = 'in_sync'`; the composite foreign key proves the binding belongs to the same Agent.
- Provider account instance is immutable. Existing Provider IDs deterministically converge on UUIDv5; local claims remain separately installation-bound.
- Dynamic route namespaces are opaque, stable across Agent rename, and invalid under another Agent audience.
- Preview rows and journal metadata contain no raw target bytes, direct file hash, local key, upstream key, OAuth token, protected slot secret, or account identifier.
- Recovery blocks all writers until every non-terminal sidecar is reconciled or explicitly classified `partial`.
- Schema, security, credential, sync, restore, and recovery work is never delegated to Kimi.

## File Responsibility Map

### Create

- `src-tauri/src/usage/agent_switch_migration.rs`
- `src-tauri/src/agent_switch/{mod.rs,domain.rs,service.rs,adapter.rs,projection.rs,guard.rs,locks.rs,route_registry.rs,target_io.rs,journal.rs,snapshot.rs,recovery.rs,source_epochs.rs,sync_guard.rs}`
- `src-tauri/src/database/dao/{agent_switch_states.rs,account_claims.rs,agent_route_namespaces.rs,custom_agent_adapters.rs,live_target_ownership.rs,agent_switch_journals.rs,source_epochs.rs,switch_sensitive_sync.rs}`
- `src-tauri/src/commands/agent_switch.rs`
- `src-tauri/tests/agent_switch_foundation_e2e.rs`

### Modify

- `src-tauri/Cargo.toml`
- `src-tauri/src/usage/mod.rs`
- `src-tauri/src/database/{mod.rs,schema.rs,tests.rs,backup.rs}`
- `src-tauri/src/database/dao/mod.rs`
- `src-tauri/src/usage/{domain.rs,ingestion.rs,quota.rs}`
- `src-tauri/src/database/dao/{usage_events.rs,quota.rs,usage_providers.rs,agent_modules.rs,agent_provider_bindings.rs}`
- `src-tauri/src/credentials/{mod.rs,service.rs,lifecycle_lock.rs}`
- `src-tauri/src/proxy/{binding_auth.rs,server.rs,handler_context.rs,provider_router.rs,types.rs}`
- `src-tauri/src/services/{sync_protocol.rs,webdav_sync.rs,s3_sync.rs}`
- `src-tauri/src/commands/{mod.rs,usage_dashboard.rs,import_export.rs,webdav_sync.rs,s3_sync.rs}`
- `src-tauri/src/{store.rs,lib.rs}`

## Task 1: Add the physical v18→v19 migration and complete schema

**Files:**

- Create: `src-tauri/src/usage/agent_switch_migration.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/database/{mod.rs,schema.rs,tests.rs,backup.rs}`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Test: `src-tauri/src/database/tests.rs`
- Test: `src-tauri/src/database/backup.rs`

**Produces:** `SCHEMA_VERSION = 19`, a transactional v18→v19 migration, and one exact completeness validator used by startup, import, and restore.

- [ ] **Step 1: Write failing v18 fixture, migration, convergence, and rollback tests**

Build the v18 fixture through the real migrations, assert `daily_budget_usd` exists, insert representative built-in/custom Providers, Agent bindings, source pointers, events, quota, and protected credential metadata, then migrate and verify:

```rust
assert_eq!(Database::get_user_version(&conn)?, 19);
assert!(Database::has_column(&conn, "usage_providers", "daily_budget_usd")?);
assert!(Database::has_column(&conn, "usage_providers", "account_instance_id")?);
assert!(Database::has_column(&conn, "agent_provider_bindings", "default_model")?);
assert!(Database::has_column(&conn, "usage_events", "account_claim_id")?);
assert!(Database::has_column(&conn, "quota_snapshots", "account_claim_id")?);
```

Also prove:

- the same Provider ID in two independent v18 fixtures derives the same UUIDv5;
- two different Provider IDs derive different account instances;
- v18 budget values and all logical Provider/binding/event/quota IDs survive;
- old event/quota account fields remain null;
- a forced late DDL failure rolls back all v19 tables, rebuilt tables, triggers, indexes, and `user_version`;
- a database claiming v19 but missing one required table, column, trigger, unique index, or FK fails `validate_schema_v19_complete`;
- running initialization twice is idempotent.

- [ ] **Step 2: Run the migration tests and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml migration_v18_to_v19 --lib -- --nocapture
```

Expected: FAIL because schema v19 and its validator do not exist.

- [ ] **Step 3: Add the required cryptography and UUID capabilities**

Change only these dependency entries:

```toml
uuid = { version = "1.11", features = ["v4", "v5"] }
chacha20poly1305 = { version = "0.10", features = ["std"] }
```

Run `pnpm install --offline` only if the workspace wrapper needs to refresh metadata. Review `Cargo.lock` completely for the new transitive set; do not upgrade unrelated crates.

- [ ] **Step 4: Implement the v19 migration entry points**

Export this exact surface:

```rust
pub(crate) const ACCOUNT_INSTANCE_NAMESPACE: Uuid =
    uuid::uuid!("8f314dc2-e39d-5fd4-91b8-bcb20d0bd2ee");

pub(crate) fn migrate_v18_to_v19(conn: &mut Connection) -> Result<(), AppError>;
pub(crate) fn validate_schema_v19_complete(conn: &Connection) -> Result<(), AppError>;

pub(crate) fn migrated_account_instance_id(provider_id: &str) -> String {
    Uuid::new_v5(&ACCOUNT_INSTANCE_NAMESPACE, provider_id.as_bytes()).to_string()
}
```

The migration begins one SQLite transaction, rebuilds `usage_providers` to make `account_instance_id` truly non-null/unique while retaining every v18 column including `daily_budget_usd`, restores the v17 system identity/delete triggers, adds the binding/event/quota columns, creates all tables below, seeds metadata, validates inside the transaction, sets `user_version = 19`, and commits once.

Existing-table additions:

```text
usage_providers:
  account_instance_id TEXT NOT NULL UNIQUE
  catalog_origin_provider_id TEXT NULL REFERENCES usage_providers(id) ON DELETE RESTRICT
  credential_mode TEXT NOT NULL CHECK(official|provider_split|legacy_combined)
  UNIQUE(id,account_instance_id) for claim provenance foreign keys

agent_provider_bindings:
  default_model TEXT NULL
  upstream_model TEXT NULL
  switch_config_version INTEGER NOT NULL DEFAULT 0 CHECK >= 0
  credential_mode TEXT NOT NULL CHECK(official|provider_split|legacy_combined)
  UNIQUE(agent_module_id,id)

usage_events / quota_snapshots:
  account_instance_id TEXT NULL
  account_claim_id TEXT NULL
  account_provenance TEXT NULL CHECK(verified|unproven|conflict|legacy_unscoped)
```

Create the following tables with foreign keys and CHECK constraints matching the approved design:

```text
agent_switch_states
provider_account_claims
agent_route_namespaces
custom_agent_adapters
custom_agent_adapter_targets
custom_agent_adapter_mappings
custom_agent_adapter_drafts
agent_live_target_ownership
agent_switch_previews
agent_switch_journals
agent_switch_journal_targets
usage_source_epochs
switch_sensitive_pending
switch_sensitive_rejections
```

Use these exact persistence responsibilities; JSON columns contain only the named non-secret structured values and are validated on read/write:

```text
provider_account_claims:
  account_claim_id PK, provider_id, account_instance_id, origin_installation_id,
  claim_generation, credential_kind, credential_generation, proof_status,
  nullable opaque account_proof_token, active, created_at, updated_at
  FK (provider_id,account_instance_id) -> usage_providers(id,account_instance_id)

agent_route_namespaces:
  agent_module_id PK, opaque_namespace UNIQUE, protocol_family,
  origin_generation, installation_id, created_at, updated_at

custom_agent_adapters:
  agent_module_id PK, protocol_family, adapter_version, adapter_schema_version,
  validated_target_set_digest, installation_id, created_at, updated_at

custom_agent_adapter_targets:
  target_id PK, agent_module_id, ordinal, display_path, canonical_target_identity,
  format, create_missing, target_version, created_at, updated_at
  UNIQUE(agent_module_id,ordinal), UNIQUE(canonical_target_identity)

custom_agent_adapter_mappings:
  mapping_id PK, agent_module_id, target_id, ordinal, logical_value,
  literal_segments_json, created_at, updated_at
  UNIQUE(target_id,ordinal), UNIQUE(target_id,literal_segments_json)

custom_agent_adapter_drafts:
  agent_module_id PK, draft_kind, nullable base_adapter_version, wizard_step,
  non_secret_form_json, validation_messages_json, draft_version,
  installation_id, created_at, updated_at

agent_live_target_ownership:
  canonical_target_identity PK, agent_module_id, adapter_version,
  ownership_generation, owned_fields_json, installation_id, created_at, updated_at

agent_switch_previews:
  preview_token_fingerprint PK, agent_module_id, binding_id,
  expected_versions_json, intended_selection_json, normalized_digest_hmac,
  redacted_diff_json, confirmation_reasons_json, expires_at, consumed_at,
  installation_id, created_at

agent_switch_journals:
  journal_id PK, nullable parent_journal_id, transaction_kind, agent_module_id,
  phase, recorded_pre_state_version, intended_post_state_version,
  expected_generations_json, intended_selection_json, recovery_status,
  created_at, stale_diagnostic_at, updated_at

agent_switch_journal_targets:
  journal_id, target_ordinal, canonical_target_identity,
  before_digest_hmac, intended_digest_hmac, snapshot_blob_id,
  file_commit_marker, created_target, PRIMARY KEY(journal_id,target_ordinal)

usage_source_epochs:
  epoch_id PK, source, agent_module_id, provider_id, valid_from, nullable valid_to,
  account_instance_id, nullable account_claim_id, provenance,
  switch_state_version, installation_id, created_at
  one partial unique open-epoch index per (source,agent_module_id)

switch_sensitive_pending:
  mutation_id PK, causal_sync_version, source_installation_label,
  mutation_kind, redacted_summary_json, affected_agent_ids_json,
  blocker_codes_json, allowed_actions_json, logical_patch_json,
  installation_id, created_at, updated_at

switch_sensitive_rejections:
  mutation_id, causal_sync_version, installation_id, rejected_at,
  PRIMARY KEY(mutation_id,causal_sync_version,installation_id)
```

`display_path`, Custom adapter rows, claims, epochs, previews, journals, pending rows, and ownership rows are installation-local and excluded from logical export/sync/normal backup. `logical_patch_json` contains only the staged logical metadata delta and can be applied only after a fresh guarded resolution; it never contains a credential, configured file bytes, local target identity, or direct digest. Persist only an HMAC/fingerprint of the preview token, not the bearer token returned to the caller.

Minimum switch-state schema:

```sql
CREATE TABLE agent_switch_states (
  agent_module_id TEXT PRIMARY KEY REFERENCES agent_modules(id) ON DELETE RESTRICT,
  current_binding_id TEXT,
  last_verified_binding_id TEXT,
  config_state TEXT NOT NULL CHECK (config_state IN
    ('in_sync','drifted','missing','setup_required','unmanaged','partial')),
  route_health TEXT NOT NULL CHECK (route_health IN
    ('usable','unknown','proxy_down','auth_missing','credential_unavailable','upstream_unhealthy')),
  route_health_reason TEXT,
  state_version INTEGER NOT NULL CHECK (state_version >= 1),
  adapter_kind TEXT NOT NULL,
  adapter_schema_version INTEGER NOT NULL CHECK (adapter_schema_version >= 0),
  observed_digest BLOB,
  verified_digest BLOB,
  selection_provenance TEXT CHECK
    (selection_provenance IS NULL OR selection_provenance IN ('applied','detected_external')),
  last_observed_at INTEGER,
  last_verified_at INTEGER,
  last_health_check_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (agent_module_id,current_binding_id)
    REFERENCES agent_provider_bindings(agent_module_id,id) ON DELETE RESTRICT,
  CHECK ((config_state = 'in_sync' AND current_binding_id IS NOT NULL)
      OR (config_state <> 'in_sync' AND current_binding_id IS NULL))
);
```

Use triggers to prevent updates to `usage_providers.account_instance_id`, `usage_providers.catalog_origin_provider_id` through generic SQL update paths, append-only mutation of closed source epochs, and deletion of a non-terminal journal. The dedicated account service may use a transaction-scoped internal insertion path; generic `save_usage_provider` never accepts either identity field.

- [ ] **Step 5: Seed conservative migration metadata**

- Fixed system API Providers and their bindings become `provider_split`.
- Subscription Providers and bindings become `official`; they retain their auth kind and receive account instances/claims without being treated as API split credentials.
- Existing custom API Providers/bindings become `legacy_combined`.
- Existing event/quota account fields stay null. Read projection maps the all-null provenance tuple to `legacy_unscoped`; migration does not backfill that label or any guessed identity into historical rows.
- Every existing binding keeps `default_model` and `upstream_model` null. Migration never guesses a model from Provider metadata or a mutable live file; the user must choose one or explicitly adopt an exact detector-observed model before first switch.
- Existing protected local credentials and detectable official logins receive random installation-local claims with `proof_status = 'unproven'`.
- Existing source pointers open one epoch at migration time; no earlier event is assigned to it.
- Built-in Agent state rows are non-current `unmanaged` pending detector observation; existing Custom Agents are `setup_required`.
- Migration creates no adapter path, no current binding, and no route namespace usable by the proxy until registry initialization validates it.

- [ ] **Step 6: Wire dispatch and all validator call sites**

Set `SCHEMA_VERSION` to 19, add only the `18 => migrate_v18_to_v19` branch after the menu migration, and replace restore/import assumptions that hard-code v17/v18 completeness with `validate_schema_v19_complete`. A v18 app must still reject a v19 database as too new.

- [ ] **Step 7: Run focused and full migration gates**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml migration_v18_to_v19 --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::backup --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests --lib
```

Expected: all pass, including v18 budget preservation and transaction rollback.

- [ ] **Step 8: Commit the migration alone**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock \
  src-tauri/src/usage/agent_switch_migration.rs src-tauri/src/usage/mod.rs \
  src-tauri/src/database/mod.rs src-tauri/src/database/schema.rs \
  src-tauri/src/database/tests.rs src-tauri/src/database/backup.rs \
  src-tauri/src/database/dao/mod.rs
git diff --cached --check
git commit -m "feat: add agent switching schema v19"
```

## Task 2: Implement state, account, route, adapter-metadata, and source-epoch DAOs

**Files:**

- Create: `src-tauri/src/agent_switch/{mod.rs,domain.rs,route_registry.rs,source_epochs.rs}`
- Create: `src-tauri/src/database/dao/{agent_switch_states.rs,account_claims.rs,agent_route_namespaces.rs,custom_agent_adapters.rs,source_epochs.rs}`
- Modify: `src-tauri/src/database/dao/{mod.rs,usage_providers.rs,agent_modules.rs,agent_provider_bindings.rs,usage_events.rs,quota.rs}`
- Modify: `src-tauri/src/usage/domain.rs`
- Test: each new DAO file plus `src-tauri/src/database/tests.rs`

**Produces:** typed monotonic persistence APIs; it does not yet write live files.

- [ ] **Step 1: Write failing constraint and compare-and-swap tests**

Cover:

- exactly one permanent state per Agent and no ABA return to a missing row;
- only same-Agent binding may become current;
- only exact expected `state_version` can publish or clear current;
- drift/missing/setup/unmanaged/partial clears current and retains last-verified only as explanation;
- a current binding cannot be deleted until current and last-verified references are safely handled;
- account instance immutability and catalog-origin validation;
- UUIDv5 migration convergence versus random UUIDv4 for new Providers;
- claim generation/proof/conflict transitions and no raw proof source in SQL;
- stable namespace across Agent rename, unique opaque segment, and protocol validation;
- append-only source epochs, no overlaps, explicit transition gap, occurrence-time selection, and ambiguous/no-epoch warning;
- adapter/draft version compare-and-swap and at most one draft per Agent.

- [ ] **Step 2: Run the focused tests and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch_state --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml account_claim --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml route_namespace --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml source_epoch --lib -- --nocapture
```

- [ ] **Step 3: Define the shared domain without secret-bearing fields**

Implement the enums from the master plan plus:

```rust
pub struct AgentSwitchStateView {
    pub agent_module_id: String,
    pub current_binding_id: Option<String>,
    pub last_verified_binding_id: Option<String>,
    pub config_state: AgentConfigState,
    pub route_health: AgentRouteHealth,
    pub route_health_reason: Option<String>,
    pub state_version: u64,
    pub adapter_kind: String,
    pub adapter_version: u64,
    pub activity: Option<AgentSwitchActivity>,
    pub selection_provenance: Option<SelectionProvenance>,
    pub observed_at: Option<i64>,
    pub verified_at: Option<i64>,
}

pub struct FrozenAccountProvenance {
    pub provider_id: String,
    pub account_instance_id: String,
    pub account_claim_id: Option<String>,
    pub provenance: AccountProvenance,
}
```

Digest bytes, proof tokens, installation ID, slots, journal paths, and target identities are private DAO/service structs and never serialized.

- [ ] **Step 4: Implement the DAO transaction surfaces**

Use narrow transaction-bound methods, including:

```rust
pub(crate) fn compare_and_swap_current(
    tx: &Transaction<'_>,
    agent_id: &str,
    expected_version: u64,
    next: &VerifiedCurrentState,
) -> Result<AgentSwitchStateRecord, AppError>;

pub(crate) fn clear_current_with_state(
    tx: &Transaction<'_>,
    agent_id: &str,
    expected_version: u64,
    next_state: AgentConfigState,
    reason: Option<&str>,
) -> Result<AgentSwitchStateRecord, AppError>;

pub(crate) fn select_source_epoch(
    &self,
    source: &str,
    occurred_at: i64,
) -> Result<SourceEpochSelection, AppError>;
```

No DAO performs file I/O, Keychain reads, proxy start/stop, network calls, or UI publication.

- [ ] **Step 5: Implement dedicated separate-account creation**

Add a service-owned method that clones immutable catalog family/route/billing/auth metadata, creates a new Provider ID and UUIDv4 account instance, leaves `system_preset_key = NULL`, sets validated `catalog_origin_provider_id`, and creates no credential or binding implicitly. Reject arbitrary custom Providers pretending to have a catalog origin.

New custom API Providers created after v19 use `provider_split` from the start: create Provider-scoped protected credential metadata for the upstream account, and create an independent generated local credential for each later binding. Only migrated pre-v19 custom rows may begin `legacy_combined`; a new save path must never recreate combined mode.

- [ ] **Step 6: Run DAO tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch_state --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml account_claim --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml route_namespace --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml source_epoch --lib
git add src-tauri/src/agent_switch src-tauri/src/database/dao src-tauri/src/usage/domain.rs
git diff --cached --check
git commit -m "feat: persist agent switch state and provenance"
```

## Task 3: Freeze route audience and account provenance before upstream I/O

**Files:**

- Modify: `src-tauri/src/proxy/{binding_auth.rs,server.rs,handler_context.rs,provider_router.rs,types.rs}`
- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/usage/{domain.rs,ingestion.rs,quota.rs}`
- Modify: `src-tauri/src/database/dao/{usage_events.rs,quota.rs}`
- Test: the same modules
- Test: `src-tauri/tests/agent_switch_foundation_e2e.rs`

**Produces:** authorization-linearized Agent audience and immutable account provenance for all new proxy/quota evidence.

- [ ] **Step 1: Write failing dynamic-audience and provenance tests**

Prove:

- a valid binding-local key under its fixed compatibility alias succeeds;
- the same key under another fixed or dynamic Agent namespace returns the generic local authorization failure;
- a Custom namespace continues working after Agent rename;
- malformed/unknown namespace, Agent/protocol mismatch, disabled route, missing claim, or rotated key causes zero upstream attempts and zero fallback;
- one successful request freezes Agent, Provider, account instance, and claim into its `RequestContext` and `UsageEvent` even if current state changes before response ingestion;
- quota refresh freezes the same Provider/account/claim before network I/O;
- old rows with null provenance serialize as `legacy_unscoped` and are never enriched by a later join.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml route_audience --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml frozen_account_provenance --lib -- --nocapture
```

- [ ] **Step 3: Replace protocol-only resolution with route-audience resolution**

Return one protected object from credential resolution:

```rust
pub struct ResolvedBindingRoute {
    pub binding_id: String,
    pub agent_module_id: String,
    pub provider_id: String,
    pub account_instance_id: String,
    pub account_claim_id: Option<String>,
    pub route_audience: String,
    pub agent_protocol: String,
    pub upstream_protocol: String,
    pub credential_generation: u64,
    pub upstream: ResolvedUpstreamCredential,
}
```

Raw credentials stay in non-serializable/zeroizing wrappers. Resolve immediately before request construction, require the parsed path audience to equal `route_audience`, strip all inbound credential carriers, release DB/credential locks, then perform upstream I/O.

- [ ] **Step 4: Thread frozen provenance through request, usage, and quota paths**

Extend `FrozenUsageProviderContext`, `UsageIngestionInput`, `build_event`, quota observation insert/read, and safe views. Never derive event provenance from the Provider row after the request.

- [ ] **Step 5: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml route_audience --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml frozen_account_provenance --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_foundation_e2e route_audience -- --nocapture
git add src-tauri/src/proxy src-tauri/src/credentials/service.rs \
  src-tauri/src/usage src-tauri/src/database/dao/usage_events.rs \
  src-tauri/src/database/dao/quota.rs src-tauri/tests/agent_switch_foundation_e2e.rs
git diff --cached --check
git commit -m "feat: enforce agent route audiences and account provenance"
```

## Task 4: Implement authoritative switchability and guarded mutations

**Files:**

- Create: `src-tauri/src/agent_switch/{guard.rs,service.rs}`
- Create: `src-tauri/src/database/dao/live_target_ownership.rs`
- Modify: `src-tauri/src/database/dao/{agent_modules.rs,agent_provider_bindings.rs,usage_providers.rs}`
- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Test: all modified modules

**Produces:** backend-authored switchability and one guard path for disable/delete/unbind/archive/model/credential/login/proxy/import mutations.

- [ ] **Step 1: Write failing reason-order and bypass tests**

Table-drive every stable reason/action from the master plan. Assert the backend ordering and that a direct legacy command/DAO attempt cannot:

- disable, unbind, delete, archive, clear/rotate a selected credential, or mutate a current model outside coordinator reapply;
- mutate a binding that is last-verified under drift/missing/partial without guarded Disconnect/recovery;
- globally disable/delete/clear/logout a Provider without listing all affected current or uncertain Agents;
- stop/move the proxy while any current/recovery route depends on it;
- publish a switchable result for a missing model, incompatible protocol, unsupported official bridge, account conflict, or legacy-combined credential.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switchability --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml guarded_mutation --lib -- --nocapture
```

- [ ] **Step 3: Implement switchability as a pure ordered projector**

`BindingSwitchability` must be built from one coherent backend observation and include every evaluated version. A ready non-current binding returns no reasons/action `switch`; a ready current binding returns no reasons/action `reapply`; confirmation returns `preview_switch`; blocked uses the first actionable ordered reason.

- [ ] **Step 4: Route all old mutation entry points through `guard_mutation`**

Add a permit type with private fields and no public constructor. Low-level mutation functions accept that permit when the target may be current/uncertain; command-only checks are insufficient. Guard inspection returns redacted affected-Agent DTOs, never ownership digests or configuration bytes.

- [ ] **Step 5: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switchability --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml guarded_mutation --lib
git add src-tauri/src/agent_switch src-tauri/src/database/dao \
  src-tauri/src/credentials/service.rs src-tauri/src/commands/usage_dashboard.rs
git diff --cached --check
git commit -m "feat: guard current agent provider mutations"
```

## Task 5: Add stable locks, opaque previews, and live ownership

**Files:**

- Create: `src-tauri/src/agent_switch/{locks.rs,adapter.rs,projection.rs,target_io.rs}`
- Create: `src-tauri/src/database/dao/live_target_ownership.rs`
- Modify: `src-tauri/src/credentials/lifecycle_lock.rs`
- Test: new modules

**Produces:** no live writes yet; it establishes serialization, safe inspection handles, and short-lived single-use preview capability.

- [ ] **Step 1: Write failing concurrency, token, and ownership tests**

Cover sorted lock order, unrelated-Agent concurrency, same-Agent exclusion, target collision, stale generation, token expiry/single-use, digest invalidation, cross-installation token rejection, takeover/legacy ownership conflict, hardlink/symlink/special-file/file-ID/parent replacement rejection, 4 MiB per-file and 16 MiB transaction limits, and macOS-only platform gating.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_lock_order --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_preview_token --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml target_identity --lib -- --nocapture
```

- [ ] **Step 3: Implement the fixed lock order**

Acquire only in this order:

```text
installation recovery gate
global lifecycle/source gate when needed
sorted Agent IDs
sorted canonical target identities
credential lifecycle reservation
short SQLite transaction / compare-and-swap
```

Never hold SQLite or credential-store locks across parsing, fsync, bridge calls, or network I/O.

- [ ] **Step 4: Implement descriptor-safe target inspection and ownership**

Only a leading literal `~` is expanded. Reject relative traversal, variables, substitutions, denied roots, non-user-owned files, symlinks, hardlinks, special files, unsupported flags/ACLs, and size overflow. Use opened-parent/no-follow/file-ID checks for both inspection and later commit. Do not use the existing general `config::atomic_write` for managed target writes.

- [ ] **Step 5: Implement opaque preview reservation**

Persist only random token ID, installation, Agent/binding, expected versions/generations, keyed normalized digest, redacted diff, confirmation reasons, expiry, and consumed marker. Preview never exposes a digest or file path identity that can be correlated with secret-bearing bytes.

- [ ] **Step 6: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_lock_order --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_preview_token --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml target_identity --lib
git add src-tauri/src/agent_switch src-tauri/src/database/dao/live_target_ownership.rs \
  src-tauri/src/credentials/lifecycle_lock.rs
git diff --cached --check
git commit -m "feat: add safe agent switch previews and ownership"
```

## Task 6: Implement encrypted snapshots, monotonic journal, and startup recovery

**Files:**

- Create: `src-tauri/src/agent_switch/{journal.rs,snapshot.rs,recovery.rs}`
- Create: `src-tauri/src/database/dao/agent_switch_journals.rs`
- Modify: `src-tauri/src/credentials/{mod.rs,service.rs}`
- Modify: `src-tauri/src/store.rs`
- Test: new modules and `src-tauri/tests/agent_switch_foundation_e2e.rs`

**Produces:** protected before-images and fail-closed recovery primitives; adapters use them in the next child plan.

- [ ] **Step 1: Write failing phase, crypto, cleanup, and recovery tests**

Inject failpoints at reserve, key stage, each blob write/fsync, snapshot verification, sidecar phase write, target marker, DB commit, external finalization, terminal cleanup, plus protected-store loss, blob loss/tamper, sidecar lag, database replacement, unknown file edit, stale journal, and cleanup retry.

Assert:

- no target can change before `snapshot_ready`;
- nonce is unique per blob and associated data binds installation/journal/Agent/target/metadata/digest;
- sidecar is fsynced at every phase;
- post-DB-commit recovery never restores old files;
- unknown target bytes or generation mismatch enters `partial` without overwrite;
- non-terminal material is never age-deleted;
- terminal cleanup removes blob, sidecar, journal, and protected key only after durable completion;
- no raw bytes or sentinel secret appears in SQL, logs, diagnostics, or serialized DTOs.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_journal --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_snapshot --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_recovery --lib -- --nocapture
```

- [ ] **Step 3: Implement the exact journal phase machine**

```rust
pub enum JournalPhase {
    Reserved,
    SnapshotReady,
    ExternalApplied,
    TargetsCommitted,
    DbCommitted,
    ExternalFinalized,
    TerminalCleanup,
    Complete,
    Partial,
}
```

Only explicit adjacent/idempotent transitions are accepted. File contents never infer a missing phase. Record separate pre/post versions and generations.

- [ ] **Step 4: Implement XChaCha20-Poly1305 storage**

Generate one random 256-bit transaction key, store it under a fresh protected slot, encrypt each byte-exact before-image independently with unique nonce/AAD, use installation-secret HMAC for digests, write blobs and authenticated sidecar to a user-only no-backup/no-sync recovery root, fsync file and directory, decrypt/verify all blobs, then mark `snapshot_ready`.

- [ ] **Step 5: Implement startup recovery classification**

Before enabling any writer, scan sidecars and DB journals, acquire normal locks, compare the database with exact recorded pre/post state, and choose only the approved forward/rollback/post-commit/partial paths. A sidecar without a matching DB cannot assert post-state; it blocks writers for diagnosis.

- [ ] **Step 6: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_journal --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_snapshot --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_recovery --lib
git add src-tauri/src/agent_switch src-tauri/src/database/dao/agent_switch_journals.rs \
  src-tauri/src/credentials src-tauri/src/store.rs src-tauri/tests/agent_switch_foundation_e2e.rs
git diff --cached --check
git commit -m "feat: recover protected agent switch transactions"
```

## Task 7: Guard sync, import, restore, and startup ordering

**Files:**

- Create: `src-tauri/src/agent_switch/sync_guard.rs`
- Create: `src-tauri/src/database/dao/switch_sensitive_sync.rs`
- Modify: `src-tauri/src/database/backup.rs`
- Modify: `src-tauri/src/services/{sync_protocol.rs,webdav_sync.rs,s3_sync.rs}`
- Modify: `src-tauri/src/commands/{import_export.rs,webdav_sync.rs,s3_sync.rs}`
- Modify: `src-tauri/src/{store.rs,lib.rs}`
- Test: all modified modules

**Produces:** machine-local exclusion, staged remote conflicts, unforgeable restore permit, foreign recovery quarantine, and recovery-first startup.

- [ ] **Step 1: Write failing sync/restore/startup tests**

Cover:

- all local switch/adapters/paths/routes/epochs/previews/journals/recovery/ownership tables are skipped and preserved;
- remote disable/delete/unbind/default-model/account replacement affecting current, last-verified, or partial Agent becomes `switch_sensitive_pending` instead of applying;
- safe display metadata can merge;
- `keep_local` records mutation ID + causal version and prevents exact requeue while allowing a newer conflict;
- mismatched/unprovable account claims quarantine their usage/quota envelopes and allow only separate-account or keep-local;
- non-terminal local sidecar refuses SQL/binary restore at the low-level API;
- foreign recovery material is quarantined, never executed or copied over local material;
- accepted restore clears current, regenerates namespace, forces read-only detector, and performs zero configuration writes;
- startup order is sidecar scan → recovery → credential reconciliation → detector/route initialization → proxy/schedulers/config writers.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_sensitive_sync --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml restore_refuses_nonterminal_switch --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch_startup_order --lib -- --nocapture
```

- [ ] **Step 3: Add machine-local tables to both skip and preserve policy**

The staged-database replace flow must not depend on a single list: validate that local-only tables are absent from remote payload application and copied from the authoritative local DB into the staged result. Recovery filesystem roots are never inside a backup manifest.

- [ ] **Step 4: Require an unforgeable `RestorePermit` below commands**

Only `AgentSwitchService::assert_restore_allowed` constructs the private permit after scanning DB and sidecars. Every supported database replacement/import API requires `&RestorePermit`; direct internal callers cannot bypass the check.

- [ ] **Step 5: Stage switch-sensitive logical differences**

Diff logical Agent/Provider/binding/default-model/account changes before replacement. Store redacted summaries, causal version, source label, affected Agent IDs, blocker codes, and allowed actions. Applying later must re-enter normal detector/guard/preview/reapply/Disconnect; it is not a delayed raw SQL update.

- [ ] **Step 6: Reorder application startup**

Construct the credential store and recovery service without starting workers, scan/recover, reconcile credentials, initialize route/detector state, then start proxy/quota/menu snapshot/background sync. A recovery error leaves writers blocked and exposes a redacted diagnostic state.

- [ ] **Step 7: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_sensitive_sync --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml restore_refuses_nonterminal_switch --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch_startup_order --lib
git add src-tauri/src/agent_switch/sync_guard.rs \
  src-tauri/src/database/dao/switch_sensitive_sync.rs src-tauri/src/database/backup.rs \
  src-tauri/src/services src-tauri/src/commands src-tauri/src/store.rs src-tauri/src/lib.rs
git diff --cached --check
git commit -m "feat: guard agent switching sync and restore"
```

## Task 8: Expose one coherent secret-safe backend observation frame

**Files:**

- Create: `src-tauri/src/commands/agent_switch.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Test: `src-tauri/src/commands/agent_switch.rs`

**Produces:** the stable IPC used by later child plans without frontend inference.

- [ ] **Step 1: Write failing serialization and coherent-frame tests**

Assert `get_agent_switch_view` returns state plus binding rows from one observation/version frame; each binding has backend-authored selection and switchability. Serialize all success/error/preview/diagnostic/event DTOs with sentinel credentials and assert no raw secret, fingerprint, digest, slot, path identity, proof source, recovery bytes, or OAuth field occurs.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::agent_switch --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch_dto_redaction --lib -- --nocapture
```

- [ ] **Step 3: Implement and register the command shell**

Register:

```text
get_agent_switch_view
refresh_agent_switch_state
preview_agent_binding_switch
get_agent_binding_switch_preview
apply_agent_binding_switch
preview_agent_disconnect
apply_agent_disconnect
preview_guarded_agent_mutation
apply_guarded_agent_mutation
list_switch_sensitive_sync_mutations
preview_switch_sensitive_sync_mutation
resolve_switch_sensitive_sync_mutation
```

Foundation commands may return a stable `not_yet_supported` result for adapter apply until the built-in/custom child plans wire writers, but they must already validate input, read coherent state, author switchability, and never expose secret material. Do not register an apply path that can mutate state without an adapter.

- [ ] **Step 4: Extend binding DTOs**

Add `default_model`, `upstream_model`, `switch_config_version`, credential mode, `selection`, adapter/protocol summary, and `switchability`. Preserve existing enabled/effective/credential semantics as separate fields.

- [ ] **Step 5: Run the foundation gate**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::agent_switch --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_foundation_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

- [ ] **Step 6: Commit and record Gate 1 evidence**

```bash
git add src-tauri/src/commands src-tauri/src/lib.rs src-tauri/src/usage/domain.rs \
  src-tauri/tests/agent_switch_foundation_e2e.rs
git diff --cached --check
git commit -m "feat: expose agent switching backend state"
```

Record commit hashes and passing commands in the implementation task log. Do not begin built-in adapter work if recovery, redaction, restore, or provenance has an open failure.
