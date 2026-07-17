# Built-In Agent Provider Switching Implementation Plan

> **Superseded 2026-07-17:** Do not implement. See
> [`2026-07-17-provider-only-monitoring.md`](2026-07-17-provider-only-monitoring.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Codex, Claude Code, OpenCode, OpenClaw, and Hermes perform real, verified Provider/model switches through the v19 coordinator while preserving legacy Provider/Profile/takeover behavior behind the same ownership and recovery rules.

**Architecture:** Pure built-in adapters inspect original bytes and produce one complete target output; `AgentSwitchService` alone snapshots, replaces, verifies, commits, and recovers those outputs. Official authentication uses opaque Agent-specific bridges, while linked legacy actions, Profile batches, proxy lifecycle, takeover transitions, and native right-click actions enter the same binding-centric preview/apply service.

**Tech Stack:** Rust 2021, toml_edit, serde_json/JSON5/json-five, YAML section-preserving helpers, protected credentials, Tauri native menus, existing proxy and official-login services.

**Depends on:** all gates in [`2026-07-16-agent-provider-switching-foundation.md`](2026-07-16-agent-provider-switching-foundation.md).

---

## Adapter Contract

Built-in adapters live under `src-tauri/src/agent_switch/adapters/`; official bridges live under `src-tauri/src/agent_switch/official_bridge/`. Adapters never call a legacy Provider writer and never replace a live file themselves.

```rust
pub trait AgentConfigAdapter: Send + Sync {
    fn locate(&self, ctx: &AdapterContext) -> Result<Vec<TargetSpec>, AppError>;
    fn inspect(
        &self,
        ctx: &InspectContext,
        targets: &[TargetSnapshot],
    ) -> Result<DetectedSelection, AppError>;
    fn plan(
        &self,
        ctx: &PlanContext,
        projection: &SwitchProjection,
        targets: &[TargetSnapshot],
    ) -> Result<AdapterPlan, AppError>;
    fn preview(&self, plan: &AdapterPlan) -> Result<RedactedAdapterPreview, AppError>;
    fn verify(
        &self,
        ctx: &VerifyContext,
        plan: &AdapterPlan,
        reread: &[TargetSnapshot],
    ) -> Result<Verification, AppError>;
}
```

`AdapterPlan` contains intended bytes only in a secret-scoped backend wrapper with redacted `Debug`, best-effort zeroization, and no `Serialize`. The coordinator uses foundation `TargetIo` for write/rollback. The alias is deterministic from immutable binding ID and is not derived from display name:

```rust
pub(crate) fn binding_alias(binding_id: &str) -> String {
    format!("lub_{}", stable_identifier_fragment(binding_id))
}
```

Ownership is explicit `(Agent, target identity, owned field set, generation)`. A familiar field name is not evidence that LLM Usage Bar owns its existing value.

## File Responsibility Map

### Create

- `src-tauri/src/agent_switch/adapters/{mod.rs,codex.rs,claude.rs,opencode.rs,openclaw.rs,hermes.rs,fixtures.rs}`
- `src-tauri/src/agent_switch/official_bridge/{mod.rs,codex.rs,claude.rs}`
- `src-tauri/src/agent_switch/{detector.rs,route_health.rs,legacy.rs,batch.rs,proxy_lifecycle.rs,tray_actions.rs}`
- `src-tauri/tests/built_in_agent_switch_e2e.rs`

### Modify

- `src-tauri/src/agent_switch/{mod.rs,service.rs,adapter.rs,projection.rs,recovery.rs,source_epochs.rs}`
- `src-tauri/src/{codex_config.rs,config.rs,opencode_config.rs,openclaw_config.rs,hermes_config.rs,store.rs,lib.rs,tray.rs}`
- `src-tauri/src/services/{claude_cli_auth.rs,profile.rs,proxy.rs,provider/mod.rs,provider/live.rs}`
- `src-tauri/src/proxy/providers/codex_oauth_auth.rs`
- `src-tauri/src/commands/{auth.rs,claude_cli_auth.rs,codex_oauth.rs,provider.rs,profile.rs,proxy.rs}`
- `src-tauri/src/database/dao/{providers.rs,profiles.rs,proxy.rs}`
- Post-menu baseline: `src-tauri/src/{tray_popover.rs,tray_status.rs}` and `src-tauri/src/commands/tray_usage.rs` only where named below.

## Task 1: Complete coordinator orchestration and the isolated adapter harness

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/{mod.rs,fixtures.rs}`
- Modify: `src-tauri/src/agent_switch/{mod.rs,service.rs,adapter.rs,projection.rs,recovery.rs}`
- Test: the same files
- Test: `src-tauri/tests/built_in_agent_switch_e2e.rs`

**Produces:** a registry for five built-ins and the complete transaction skeleton with an injected adapter/bridge/proxy/clock/failpoint harness.

- [ ] **Step 1: Write failing registry, projection, transaction-order, and rollback tests**

Use temporary HOME, DB, recovery root, in-memory credential store, mock upstream, deterministic clock/IDs, and injected failpoints. Prove:

- exact fixed Agent → adapter registration and no display-name lookup;
- API projection contains namespaced loopback URL, binding-local key, explicit default model, optional upstream model, route/account generations, but never upstream key in planned Agent fields;
- official projection contains an opaque bridge instruction and no OAuth/local key;
- apply rejects stale state/adapter/binding/credential/route/ownership/proxy/takeover/source generations;
- journal/sidecar/snapshot ordering precedes official bridge/file mutation;
- current state changes only after every target and bridge verifies;
- a later target failure restores byte-exact owned outputs and leaves old current;
- a post-commit failure follows forward finalization/compensating partial and never restores old files.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml coordinator_transaction_order --lib -- --nocapture
```

- [ ] **Step 3: Implement the adapter registry and transaction skeleton**

Register only stable IDs `codex`, `claude-code`, `opencode`, `openclaw`, and `hermes`. The service executes the approved 13-phase transaction, consumes the preview token once, closes source epoch/gap before externally visible mutation when applicable, verifies exact bytes plus selection, commits state/ownership/epoch/legacy projection/journal atomically, then finalizes staged protected state.

- [ ] **Step 4: Expose a test-only failpoint seam**

Use a typed enum rather than environment variables:

```rust
pub enum SwitchFailpoint {
    AfterSnapshotReady,
    AfterOfficialApply,
    AfterTargetCommit { index: usize },
    AfterTargetVerification,
    AfterDbCommit,
    BeforeExternalFinalize,
}
```

Production constructs `NoopSwitchFailpoints`; tests inject deterministic failures. No test toggles the user's real environment.

- [ ] **Step 5: Run tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml coordinator_transaction_order --lib
git add src-tauri/src/agent_switch src-tauri/tests/built_in_agent_switch_e2e.rs
git diff --cached --check
git commit -m "feat: add built-in agent switch coordinator"
```

## Task 2: Implement the Codex adapter and managed-account official bridge

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/codex.rs`
- Create: `src-tauri/src/agent_switch/official_bridge/{mod.rs,codex.rs}`
- Modify: `src-tauri/src/codex_config.rs`
- Modify: `src-tauri/src/proxy/providers/codex_oauth_auth.rs`
- Modify: `src-tauri/src/commands/{auth.rs,codex_oauth.rs}`
- Test: the same modules

**Produces:** one-file Codex API/official selection without reading or writing `auth.json` through the generic transaction.

- [ ] **Step 1: Write failing API, official, preservation, and bridge-CAS tests**

Fixtures contain unrelated TOML comments, MCP servers, unknown tables, a user-owned provider, owned/non-owned selector variants, and an `auth.json` sentinel. Assert:

- API writes one owned `model_providers.<binding-alias>` entry, local base URL, correct wire API, binding-local `experimental_bearer_token`, top-level `model_provider`, and explicit `model`;
- upstream key is absent and `auth.json` remains byte-identical;
- official removes only ownership-proven API alias/selector/token, writes explicit model, preserves unrelated TOML/MCP, and delegates account choice to bridge;
- a same-name but non-owned entry yields drift/confirmation instead of deletion;
- verify checks alias, selector, model, local-key fingerprint, ownership generation, and preservation digest;
- Codex bridge records opaque account handle + generation, applies CAS, verifies account proof, rolls back pre-commit, and finalizes post-commit without journaling tokens;
- an additional subscription account is accepted only when its ordinary Provider row has the service-owned catalog-origin link to the seeded system card and a matching local account proof; a user-authored custom Provider cannot claim bridge privileges;
- remove/default/logout commands are blocked while a current/recovery binding depends on the account.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::codex::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::official_bridge::codex::tests --lib -- --nocapture
```

- [ ] **Step 3: Extract pure Codex transforms**

Implement:

```rust
fn inspect_codex_selection(source: &str, ownership: &OwnedFields)
    -> Result<DetectedSelection, AppError>;
fn build_codex_api_source(source: &str, projection: &ApiProxyProjection)
    -> Result<String, AppError>;
fn build_codex_official_source(source: &str, projection: &OfficialModeProjection)
    -> Result<String, AppError>;
fn verify_codex_selection(source: &str, expected: &ExpectedSelection)
    -> Result<Verification, AppError>;
```

Reuse parse/validation and `toml_edit` techniques from `codex_config.rs`. Do not call `write_codex_live_atomic`, `read_codex_live_settings`, `write_codex_live_for_provider`, or legacy `LiveSnapshot::Codex` from the managed adapter.

- [ ] **Step 4: Implement bridge generation/CAS**

Extend `CodexOAuthManager` with an opaque selection generation. `prepare` is read-only; `apply` records/verifies the reserved generation; `rollback` is permitted only before DB commit and only if the current opaque handle/generation still equals the transaction output; `finalize` discards rollback state without switching accounts.

- [ ] **Step 5: Run focused/regression tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::codex::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::official_bridge::codex::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml codex_config::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test provider_service provider_service_switch_codex_preserves_oauth_and_backfills_api_key_from_live_token -- --nocapture
git add src-tauri/src/agent_switch src-tauri/src/codex_config.rs \
  src-tauri/src/proxy/providers/codex_oauth_auth.rs src-tauri/src/commands/auth.rs \
  src-tauri/src/commands/codex_oauth.rs
git diff --cached --check
git commit -m "feat: switch Codex through managed bindings"
```

## Task 3: Implement the Claude Code adapter and official CLI bridge

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/claude.rs`
- Create: `src-tauri/src/agent_switch/official_bridge/claude.rs`
- Modify: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/services/claude_cli_auth.rs`
- Modify: `src-tauri/src/commands/claude_cli_auth.rs`
- Test: the same modules

**Produces:** owned-field JSON merge and official-mode verification without reading Claude's token store.

- [ ] **Step 1: Write failing merge/preservation/official tests**

Fixtures include unrelated env, hooks, permissions, role-specific model variables, unknown properties, non-owned route values, and a fake token-store path that panics if opened. Assert API changes only owned `env.ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, and `ANTHROPIC_MODEL`; official removes only owned base URL/token and retains/writes model; all unrelated content survives; official prepare/apply never logs in/out or reads token storage; verify uses allowlisted `claude auth status` evidence and fails closed; additional official-account rows require the validated catalog-origin link and matching local proof.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::claude::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::official_bridge::claude::tests --lib -- --nocapture
```

- [ ] **Step 3: Implement pure Claude JSON merge and bridge**

Reuse only path and JSON parse primitives from `config.rs`; do not replace the whole Provider settings document through `services/provider/live.rs`. The bridge exposes opaque `prepare/apply/verify/rollback/finalize`; apply is a recorded no-op because user authentication occurred explicitly before switching, while verify checks official status/account proof.

- [ ] **Step 4: Guard logout and run tests**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::claude::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::official_bridge::claude::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::claude_cli_auth::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test provider_service provider_service_switch_claude_updates_live_and_state -- --nocapture
git add src-tauri/src/agent_switch src-tauri/src/config.rs \
  src-tauri/src/services/claude_cli_auth.rs src-tauri/src/commands/claude_cli_auth.rs
git diff --cached --check
git commit -m "feat: switch Claude Code through managed bindings"
```

## Task 4: Implement the OpenCode adapter as one selector-plus-Provider output

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/opencode.rs`
- Modify: `src-tauri/src/opencode_config.rs`
- Test: both files

- [ ] **Step 1: Write failing single-output and preservation tests**

Assert one planned output simultaneously writes `provider.<binding-alias>` with local URL/key and top-level `model = "<binding-alias>/<model>"`; preserves `$schema`, other Providers, plugins, MCP, JSON5-compatible unknown subtrees; detects non-owned collisions as drift; verifies selector/entry/model/fingerprint; never calls `set_provider` as a standalone live write.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::opencode::tests --lib -- --nocapture
```

- [ ] **Step 3: Extract and implement the pure builder**

```rust
fn build_opencode_selection_source(
    original: &str,
    projection: &ApiProxyProjection,
) -> Result<String, AppError>;
```

Parse the original accepted format, update both fields in memory, render once, reparse, and verify before returning bytes.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::opencode::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml opencode_config --lib
git add src-tauri/src/agent_switch/adapters/opencode.rs src-tauri/src/opencode_config.rs
git diff --cached --check
git commit -m "feat: switch OpenCode through managed bindings"
```

## Task 5: Implement the OpenClaw adapter with one round-trip AST commit

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/openclaw.rs`
- Modify: `src-tauri/src/openclaw_config.rs`
- Test: both files

- [ ] **Step 1: Write failing combined-output/conflict tests**

Assert one document update writes `models.providers.<binding-alias>` and `agents.defaults.model.primary`, while preserving comments, fallbacks, other Agents/tools/Providers, formatting supported by `json-five`, and unknown fields. An external edit between inspect and apply must yield a stale preview without writing. Sequential `set_provider` then `set_default_model` is forbidden.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::openclaw::tests --lib -- --nocapture
```

- [ ] **Step 3: Expose pure AST load/render and implement adapter**

Refactor `OpenClawConfigDocument` so tests/adapters can load from source and render without saving. Perform both updates in one in-memory document; coordinator makes one replacement.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::openclaw::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml openclaw_config::tests --lib
git add src-tauri/src/agent_switch/adapters/openclaw.rs src-tauri/src/openclaw_config.rs
git diff --cached --check
git commit -m "feat: switch OpenClaw through managed bindings"
```

## Task 6: Implement the Hermes adapter with one YAML target output

**Files:**

- Create: `src-tauri/src/agent_switch/adapters/hermes.rs`
- Modify: `src-tauri/src/hermes_config.rs`
- Test: both files and `src-tauri/tests/hermes_roundtrip.rs`

- [ ] **Step 1: Write failing combined-output/preservation tests**

Assert one output updates the owned `custom_providers` entry plus `model.provider` and `model.default`; verifies alias/endpoint/model/fingerprint; preserves unrelated YAML sections/comments and forward-compatible Provider fields; a simulated failure after the first logical edit still produces zero live writes; sequential `set_provider`/`apply_switch_defaults` is not used.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::hermes::tests --lib -- --nocapture
```

- [ ] **Step 3: Implement the in-memory combined builder**

```rust
fn build_hermes_selection_source(
    original: &str,
    projection: &ApiProxyProjection,
) -> Result<String, AppError>;
```

Reuse section-level parse/replacement and Provider normalization, but return complete bytes to the coordinator instead of writing each section.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::adapters::hermes::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml hermes_config::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test hermes_roundtrip -- --nocapture
git add src-tauri/src/agent_switch/adapters/hermes.rs src-tauri/src/hermes_config.rs \
  src-tauri/tests/hermes_roundtrip.rs
git diff --cached --check
git commit -m "feat: switch Hermes through managed bindings"
```

## Task 7: Add detector, route health, source epochs, and managed proxy lifecycle

**Files:**

- Create: `src-tauri/src/agent_switch/{detector.rs,route_health.rs,proxy_lifecycle.rs}`
- Modify: `src-tauri/src/agent_switch/{service.rs,source_epochs.rs,recovery.rs}`
- Modify: `src-tauri/src/services/{session_usage.rs,session_usage_codex.rs,proxy.rs}`
- Modify: `src-tauri/src/commands/proxy.rs`
- Modify: `src-tauri/src/{store.rs,lib.rs}`
- Test: the same modules

**Produces:** read-only external-selection adoption, explicit gaps, orthogonal health, proxy auto-start/guarded stop/batch origin moves.

- [ ] **Step 1: Write failing detector/epoch/lifecycle tests**

Cover exact recorded match, exact other-known match → `detected_external`, partial/unknown → drifted, missing, unmanaged, no historical rewrite, external observation gap, occurrence-time late import, zero attribution inside gap, proxy auto-start and sole-start compensation, startup `in_sync + proxy_down`, stop guard, multi-Agent origin batch success, all-or-old-origin rollback, staged current local-key rotation, and staged current upstream-key replacement/account continuity.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_state_detector --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml source_epoch_switch_gap --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml managed_proxy_lifecycle --lib -- --nocapture
```

- [ ] **Step 3: Implement passive detection**

Detection compares protected fingerprints and keyed normalized digests inside backend. It may update DB metadata for an exact match, but never writes config, guesses from name, adopts an unsupported login, or stretches an old epoch across an unobserved gap.

- [ ] **Step 4: Implement lifecycle and session integration**

Switch may start only a loopback proxy under the global lifecycle lock. Stop is blocked when any current/recovering API Agent depends on it. Origin move starts the new listener, previews/writes/verifies all affected Agents under a parent journal, commits together, then stops old; failure preserves old listener/config.

- [ ] **Step 5: Implement current credential mutation transactions**

Current local-key rotation stages a new generation while the old key remains valid, allows the staged key only for the same binding/journal, rewrites and verifies the live adapter selection, then promotes by compare-and-swap and revokes the old key. Recovery promotes only when intended bytes and every generation still match; otherwise it restores proven transaction output and discards staged key.

Current Provider upstream-key replacement stages a new protected slot, runs the protected connection test, and requires the existing account proof. A proof mismatch or unavailable proof cannot publish in place and returns `create_separate_account`; failure retains the old key. Same-account publish increments route generation and refreshes route health without rewriting Agent configuration. Apply the equivalent current guard to official logout/account removal.

- [ ] **Step 6: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_state_detector --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml source_epoch_switch_gap --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml managed_proxy_lifecycle --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml current_binding_credential_rotation --lib
git add src-tauri/src/agent_switch src-tauri/src/services src-tauri/src/commands/proxy.rs \
  src-tauri/src/store.rs src-tauri/src/lib.rs
git diff --cached --check
git commit -m "feat: detect and operate managed agent routes"
```

## Task 8: Route linked legacy Provider actions and takeover through the coordinator

**Files:**

- Create: `src-tauri/src/agent_switch/legacy.rs`
- Modify: `src-tauri/src/commands/provider.rs`
- Modify: `src-tauri/src/services/{provider/mod.rs,provider/live.rs,proxy.rs}`
- Modify: `src-tauri/src/database/dao/providers.rs`
- Modify: `src-tauri/src/commands/proxy.rs`
- Test: the same modules and existing command/service integration tests

- [ ] **Step 1: Write failing linkage/compatibility/takeover tests**

Prove explicit `legacy_app_type + legacy_provider_id` plus fixed Agent mapping and unique binding is the only linkage evidence. Linked actions enter preview/apply; managed-unlinked returns `link_required`/Disconnect with zero writes; unmanaged-unlinked retains legacy behavior and then rescans. Failure never changes legacy `is_current` early. No counterpart suppresses legacy current. Managed target blocks takeover/hot-switch. `Disable takeover and switch` restores and verifies backup before coordinator claims target; restore failure leaves takeover authoritative.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::legacy::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml managed_takeover_transition --lib -- --nocapture
```

- [ ] **Step 3: Implement `LegacySwitchGateway` and compatibility projection**

The old `ProviderService::switch_normal` remains only for unmanaged legacy targets. A managed commit updates an exclusive linked legacy marker in the same DB transaction; old live writers cannot bypass partial/ownership state or backfill coordinator-owned proxy fields.

- [ ] **Step 4: Guard every takeover/proxy writer**

Wire guards into `set_takeover_for_app`, synchronous disable, stop, crash recovery, live-backup update, hot switch, proxy config/origin update, and command wrappers. Do not report takeover hot-switch as configuration switch.

- [ ] **Step 5: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::legacy::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml managed_takeover_transition --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test provider_commands --features test-hooks -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::proxy::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test provider_service -- --nocapture
git add src-tauri/src/agent_switch/legacy.rs src-tauri/src/commands/provider.rs \
  src-tauri/src/services/provider src-tauri/src/services/proxy.rs \
  src-tauri/src/database/dao/providers.rs src-tauri/src/commands/proxy.rs
git diff --cached --check
git commit -m "feat: coordinate linked legacy agent switches"
```

## Task 9: Make managed Profile application atomic

**Files:**

- Create: `src-tauri/src/agent_switch/batch.rs`
- Modify: `src-tauri/src/services/profile.rs`
- Modify: `src-tauri/src/services/{mcp.rs,skill.rs,prompt.rs}` where Profile payload application currently writes managed targets
- Modify: `src-tauri/src/commands/profile.rs`
- Modify: `src-tauri/src/database/dao/profiles.rs`
- Modify: `src-tauri/src/tray.rs`
- Test: the same modules and profile round-trip tests

- [ ] **Step 1: Write failing all-or-rollback and compatibility tests**

Cover versioned managed binding refs plus legacy IDs, snapshot of managed state version, sorted batch locks, parent/child journal, one preview, second-child failure restoring all live bytes/current/profile/compat markers, confirmation-only preview, stale child rejection, and unchanged unmanaged legacy Profile round-trip.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::batch::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test profile_roundtrip --features test-hooks -- --nocapture
```

- [ ] **Step 3: Implement versioned preview/apply**

Replace managed best-effort application with `build_profile_preview` and `apply_profile_batch`. Every managed child verifies before one DB commit publishes current bindings/current Profile/legacy projections. Existing Profile-owned MCP, Skill, and Prompt writes that touch a managed target must be converted into reversible child plans under the same parent journal; they cannot run as direct writers and then claim batch atomicity. Any drift, target creation, takeover, or destructive confirmation returns preview to the main window. Unmanaged children retain their compatibility path but cannot make a mixed managed batch claim atomic success unless they are represented by reversible plans in the parent transaction.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml agent_switch::batch::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test profile_roundtrip --features test-hooks -- --nocapture
git add src-tauri/src/agent_switch/batch.rs src-tauri/src/services/profile.rs \
  src-tauri/src/services/mcp.rs src-tauri/src/services/skill.rs src-tauri/src/services/prompt.rs \
  src-tauri/src/commands/profile.rs src-tauri/src/database/dao/profiles.rs src-tauri/src/tray.rs
git diff --cached --check
git commit -m "feat: apply managed agent profiles atomically"
```

## Task 10: Add verified native right-click quick-switch backend behavior

**Files:**

- Create: `src-tauri/src/agent_switch/tray_actions.rs`
- Modify: post-menu `src-tauri/src/{tray.rs,tray_popover.rs,tray_status.rs,lib.rs}`
- Modify: `src-tauri/src/commands/tray_usage.rs`
- Test: the same modules

**Produces:** opaque menu-item registry, verified checkmarks/pending, quick-apply eligibility, and typed main-window fallback. The renderer handles the destination in the frontend child plan.

- [ ] **Step 1: Write failing menu-state and click tests**

Prove:

- checkmark derives only from verified managed current or unmanaged legacy current;
- drift/missing/partial shows Current unknown with no managed checkmark;
- opaque item ID resolves exact Agent/binding/versions, never display text or naked legacy ID;
- fresh `in_sync + ready` preview quick-applies; stale/confirmation/takeover/Profile batch opens typed main destination;
- old checkmark remains during pending, same-Agent conflict is disabled, unrelated Agents remain usable;
- verified commit moves checkmark only after detector rescan;
- rollback retains old; partial clears managed checkmark and persists an error attention;
- left-click remains usage-only and performs zero switch commands.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_popover::tests --lib -- --nocapture
```

- [ ] **Step 3: Implement `TraySwitchActionRegistry` and typed fallback**

Extend post-menu `MainWindowDestination` with `AgentSwitch { agent_module_id, binding_id, preview_token }`. The registry owns opaque menu IDs and frozen versions. A stale prepared preview is never displayed as current; renderer requests a fresh preview. Keep `show_menu_on_left_click(false)` and do not add switch actions to `TrayUsagePopover`.

- [ ] **Step 4: Preserve current native menu scope**

Do not add OpenCode/OpenClaw/Hermes sections merely because adapters exist. Preserve the existing right-click Provider/Profile sections unless a separately approved UX change says otherwise; linked items within existing sections use the gateway.

- [ ] **Step 5: Run Gate 2**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_popover::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_status::tests --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test built_in_agent_switch_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --tests --features test-hooks
```

- [ ] **Step 6: Commit and record Gate 2 evidence**

```bash
git add src-tauri/src/agent_switch/tray_actions.rs src-tauri/src/tray.rs \
  src-tauri/src/tray_popover.rs src-tauri/src/tray_status.rs \
  src-tauri/src/commands/tray_usage.rs src-tauri/src/lib.rs \
  src-tauri/tests/built_in_agent_switch_e2e.rs
git diff --cached --check
git commit -m "feat: quick switch verified agent providers from tray"
```

Do not begin Custom Agent implementation if any built-in adapter can partially write, if an official bridge touches client token files, if linked legacy can bypass ownership, or if Profile application remains best-effort for managed Agents.
