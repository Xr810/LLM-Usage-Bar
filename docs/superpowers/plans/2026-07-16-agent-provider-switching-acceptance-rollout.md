# Agent Provider Switching Acceptance and Rollout Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the complete first release—including Custom Agents—against configuration correctness, routing/attribution, crash recovery, account isolation, legacy compatibility, renderer/native behavior, secret boundaries, and the full repository regression suite.

**Architecture:** Dedicated isolated acceptance harnesses exercise the real v19 database, coordinator, safe target I/O, protected-store test double, dynamic proxy, mock upstream, source epochs, sync staging, and renderer IPC fixtures. A parameterized Custom Agent gate is release-blocking; final evidence maps every approved-design requirement to a named passing test or inspected runtime artifact.

**Tech Stack:** Rust integration tests, deterministic failpoints, tempfile/temporary HOME, mock Axum upstream, in-memory protected credentials, Vitest/Testing Library, Tauri command test hooks, Clippy, TypeScript, Vite build.

**Depends on:** Gate 4 in [`2026-07-16-agent-provider-switching-frontend-native.md`](2026-07-16-agent-provider-switching-frontend-native.md).

---

## Release Rule

The feature is not complete unless `first_release_custom_agent_gate` passes. Built-in adapters, a rendered Custom wizard, or a single Custom format/protocol demonstration cannot substitute for this gate. The gate is a full matrix over:

- Agent-facing protocol: Claude, Codex, Gemini;
- format: JSON, TOML, YAML, dotenv; and
- target count: single-file and multi-file.

That is 24 direct parameter cases. Format-specific unsupported constructs are separate negative cases. Every case performs real A→B→A switching through a dynamic route and validates immutable usage provenance. The implementation may optimize shared fixture setup, but may not mark most matrix entries ignored or reduce them to parser-only tests.

## Test Isolation Contract

Every Rust acceptance process must:

```rust
let home = tempfile::tempdir()?;
std::env::set_var("LLM_USAGE_BAR_TEST_HOME", home.path());
```

and use:

- a temporary v19 database and application-data/recovery root;
- injected in-memory credential store with secret sentinels;
- deterministic installation/clock/ID/failpoint sources;
- loopback mock upstream and loopback proxy on ephemeral ports;
- no client-owned token file, real Keychain, real Agent config, real sync endpoint, or developer App data;
- serial isolation where environment variables or process-global route hooks are involved.

Tests restore process-global state on drop. Never run the desktop app against the developer's actual HOME to prove this feature.

## File Responsibility Map

### Create

- `src-tauri/tests/agent_switch_acceptance.rs`
- `src-tauri/tests/agent_switch_recovery_matrix.rs`
- `src-tauri/tests/agent_switch_account_sync.rs`
- `src-tauri/tests/agent_switch_security.rs`
- `tests/integration/AgentProviderSwitching.test.tsx`
- `scripts/check-agent-switch-secret-boundaries.mjs`
- `docs/superpowers/progress/2026-07-16-agent-provider-switching-completion-audit.md`

### Modify

- `src-tauri/tests/{custom_agent_switch_e2e.rs,built_in_agent_switch_e2e.rs,usage_dashboard_proxy_e2e.rs}`
- Existing focused Rust tests named throughout the child plans
- Existing frontend tests named in the frontend child plan
- `package.json` only if an exact reusable verification script is added; do not add a dependency.

## Task 1: Build the release-blocking parameterized Custom Agent gate

**Files:**

- Create: `src-tauri/tests/agent_switch_acceptance.rs`
- Modify: `src-tauri/tests/custom_agent_switch_e2e.rs`
- Modify: `src-tauri/tests/usage_dashboard_proxy_e2e.rs`

**Produces:** one named gate that directly proves the approved 16-step Custom Agent workflow over 24 protocol/format/target combinations.

- [ ] **Step 1: Write the matrix definition and fail if any case is skipped**

Use explicit enum products and assert the expected cardinality:

```rust
let cases = ProtocolFamily::ALL
    .into_iter()
    .flat_map(|protocol| {
        AdapterFormat::ALL.into_iter().flat_map(move |format| {
            TargetShape::ALL
                .into_iter()
                .map(move |shape| AcceptanceCase { protocol, format, shape })
        })
    })
    .collect::<Vec<_>>();
assert_eq!(cases.len(), 24);
```

Do not use ignored tests or platform skips on macOS. Non-macOS CI may assert the feature is unavailable until equivalent target primitives exist, but the macOS release lane must execute all 24.

- [ ] **Step 2: Implement workflow stage 1—cancel, draft, resume, and setup-required**

For every case:

1. open unsaved creation state in the harness, cancel, and assert no Agent/draft/state/route/ownership/file;
2. Save draft and assert atomic Custom Agent + permanent `setup_required` state + create draft;
3. assert zero configuration writes and no proxy-usable route;
4. close/reopen and assert exact saved step/data/version;
5. discard a separate create-draft fixture and prove Agent remains setup-required while file remains untouched.

- [ ] **Step 3: Implement workflow stage 2—validation and missing-target confirmation**

For each format, generate accepted unrelated content. Single-file maps all required logical roles; multi-file distributes them and repeats at least one logical value. Include one missing target with selected existing parent. Validate, obtain redacted preview, cancel it, and assert zero bytes/files/ownership changed. Then re-preview and explicitly confirm creation.

- [ ] **Step 4: Implement workflow stage 3—two split Providers and models**

Create two protocol-compatible API Providers with distinct account instances, Provider-scoped upstream credential sentinels, and proof/claims. Bind both to the Custom Agent with independent local credentials. Set explicit Agent-facing default and upstream model on each. Assert subscriptions and incompatible protocols remain blocked with backend reason.

- [ ] **Step 5: Implement workflow stage 4—A→B→A real switching**

After every switch, assert:

- target base URL selects the immutable dynamic Agent route;
- selected local-key fingerprint matches the intended binding inside backend only;
- selector/default model and upstream mapping match;
- switch-state current binding, state version, adapter version, ownership generation, and source epoch match;
- `config_state = in_sync` and `route_health = usable`;
- unrelated keys/comments/Provider entries and supported owner/mode/metadata remain unchanged;
- no renderer/DTO includes raw local/upstream secret.

- [ ] **Step 6: Implement workflow stage 5—request routing and immutable attribution**

Send one real proxy request after A, B, and A. Mock upstream asserts the correct Provider-scoped upstream credential/model while Agent file contains only the local key. Persisted event asserts stable Custom Agent ID, Provider ID, account instance, account claim, and occurrence time. Switch again and prove old events do not move.

- [ ] **Step 7: Implement workflow stage 6—rename and cross-audience replay rejection**

Rename the Custom Agent, assert route namespace/path/generation stability, then present its local key under a different fixed/dynamic Agent namespace. Assert generic local auth failure, zero upstream attempts, zero fallback, and zero usage event.

- [ ] **Step 8: Implement workflow stage 7—current mutation guards**

Attempt binding disable/unbind/delete/local credential clear, Provider disable/delete/upstream clear, Agent archive/delete, adapter removal/edit without preview, proxy stop/origin move outside batch, and official/custom incompatible adoption. Assert each refuses with affected Agent/reason and current/live bytes stay unchanged.

- [ ] **Step 9: Implement workflow stage 8—synchronous multi-file rollback**

For multi-file cases inject failure on later target replacement/verification. Assert byte-exact rollback, file metadata preservation, old current/state/epoch authoritative, newly created target removed only if transaction-owned/unchanged, no partial success, and cleanup completes. Single-file cases inject after replacement before verify and assert equivalent rollback.

- [ ] **Step 10: Implement workflow stage 9—external detection, drift, Disconnect, delete**

Externally write an exact Provider B selection and detect `detected_external`; then write an unknown partial selection and detect Current unknown/drifted with last-verified explanatory only and no overwrite. Restore a known state, perform confirmed Disconnect, prove no local binding credential remains selected/current and ownership is released safely, then complete existing history-safe delete flow.

- [ ] **Step 11: Implement workflow stage 10—terminal cleanup and sentinel scan**

After success/rollback/recovery, assert terminal DB journals, encrypted blobs, sidecars, protected snapshot keys, and staged slots are removed only after cleanup succeeds. Scan safe UI payloads, collected logs, SQL rows, diagnostics, logical export/sync envelope, and recovery metadata for all local/upstream/OAuth/account-identifier sentinels.

- [ ] **Step 12: Run the gate and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_acceptance first_release_custom_agent_gate -- --nocapture
git add src-tauri/tests/agent_switch_acceptance.rs \
  src-tauri/tests/custom_agent_switch_e2e.rs \
  src-tauri/tests/usage_dashboard_proxy_e2e.rs
git diff --cached --check
git commit -m "test: gate first release on custom agent switching"
```

Expected: 24 parameter cases pass with no ignored macOS entry.

## Task 2: Prove every pre/post-commit crash-recovery branch

**Files:**

- Create: `src-tauri/tests/agent_switch_recovery_matrix.rs`
- Modify: `src-tauri/tests/agent_switch_acceptance.rs`

**Produces:** deterministic failure/crash evidence at every external phase for file, official bridge, credential rotation, adapter migration, Profile batch, and proxy-origin batch transactions.

- [ ] **Step 1: Define complete failpoint-phase coverage**

Enumerate reserve, protected key stage, each blob write/fsync, snapshot verify, sidecar `snapshot_ready`, source gap close, official apply, every target temp/write/rename/dir-fsync/marker, reread/selection verify, DB commit, sidecar DB-commit lag, credential/bridge finalization, terminal marker, and cleanup. Test suite asserts every enum variant ran at least once.

- [ ] **Step 2: Write pre-commit recovery tests**

When DB matches recorded pre-state, prove forward completion occurs only with exact installation, binding/model, Provider route, adapter/target set, credentials, proxy/takeover/ownership, intended bytes, all generations, bridge marker, and CAS. Otherwise restore only proven transaction-owned outputs and bridge state, verify exact before-images, and open a new old-Provider epoch at rollback completion.

- [ ] **Step 3: Write post-commit recovery tests**

When DB matches recorded post-state/phase, prove old files are never restored. Exact post-state finalizes staged credentials/bridge idempotently; unprovable post-state makes one compensating CAS that clears current, closes new epoch, opens an unproven gap, and enters partial without overwriting unknown bytes.

- [ ] **Step 4: Write ambiguity/tamper/sidecar tests**

Cover DB matching neither pre/post, changed inode/file ID, external edit after transaction output, missing/tampered key/blob, unsupported metadata, sidecar lag, database replacement, sidecar without DB, stale recovery diagnostics, terminal cleanup retry, and orphan removal only when snapshot-ready is provably impossible. Every uncertain case becomes partial and blocks all writers.

- [ ] **Step 5: Prove source gaps under crashes**

Insert late session records before, inside, and after transition gaps for both Claude and Codex import paths. Assert old/new epochs select by occurrence time, gap records stay unassigned with warning, rollback opens a new old epoch rather than stretching the old one, and recovery never rewrites a persisted event.

- [ ] **Step 6: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_recovery_matrix -- --nocapture
git add src-tauri/tests/agent_switch_recovery_matrix.rs src-tauri/tests/agent_switch_acceptance.rs
git diff --cached --check
git commit -m "test: exhaust agent switch crash recovery"
```

## Task 3: Prove account, sync, restore, credential-upgrade, and machine boundaries

**Files:**

- Create: `src-tauri/tests/agent_switch_account_sync.rs`
- Modify: `src-tauri/src/database/backup.rs` tests only where a low-level seam is required
- Modify: `src-tauri/src/services/sync_protocol.rs` tests only where a low-level seam is required

**Produces:** cross-installation evidence that Provider-keyed history and credentials never silently merge accounts or execute foreign local state.

- [ ] **Step 1: Write deterministic migration/account-instance tests**

Two offline v18 installations upgrading the same Provider ID derive the same UUIDv5 account instance. Local claims are distinct. Equal protected proof tokens aggregate; different/unavailable proof conflicts. A new independent custom Provider uses random UUIDv4. Account instance/catalog origin cannot be edited through generic save/import.

- [ ] **Step 2: Write claim/envelope quarantine tests**

Freeze Provider/account/claim into events/quota/sync envelopes. Mismatched/unprovable remote claim and account-sensitive evidence remain quarantined, never relabeled. Pre-account rows remain `legacy_unscoped`. Separate-account creates a new Provider/account/claim and requires explicit rebind; it does not move old events.

- [ ] **Step 3: Write switch-sensitive sync queue tests**

Remote disable/delete/unbind/default-model/account replacement affecting current/last/partial becomes a pending item. Safe display change merges. Guarded apply re-detects/previews/reapplies/Disconnects; keep-local persists causal rejection; a newer conflict may reappear; account mismatch allows only separate-account/keep-local.

- [ ] **Step 4: Write restore and foreign-local-state tests**

Non-terminal local sidecar refuses SQL/binary restore. Foreign journals/blobs/adapter paths/routes/epochs/ownership are quarantined and never executed/copied over local recovery. Accepted restore clears current, regenerates namespace, and performs read-only detector before any writer. Exact live match may be adopted only by local detector.

- [ ] **Step 5: Write legacy-combined upgrade tests**

Current/last/partial/unknown-selected binding refuses upgrade. Equal combined secrets stage one Provider upstream credential and independent local keys; mismatched secrets require separate Providers. Every failure before publish preserves old combined route; recovery completes/rolls back journal; old slot is not deleted before publish; upgrade performs no Agent config switch.

- [ ] **Step 6: Write current credential rotation/replacement tests**

For a current API binding, rotate its local key and prove old-key validity until verified promotion, staged-key confinement to the transaction/binding, live-file reapply, crash recovery, and old-key revocation only after commit. Replace its Provider upstream key and prove protected test + same-account proof, no Agent file rewrite, route-generation/health refresh, old-key preservation on failure, and separate-account-only behavior for mismatched/unavailable proof. Prove current official logout/account removal stays guarded.

- [ ] **Step 7: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_account_sync -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml restore_refuses_nonterminal_switch --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml switch_sensitive_sync --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml current_binding_credential_rotation --lib
git add src-tauri/tests/agent_switch_account_sync.rs \
  src-tauri/src/database/backup.rs src-tauri/src/services/sync_protocol.rs
git diff --cached --check
git commit -m "test: prove agent account and machine isolation"
```

## Task 4: Prove built-in, legacy, Profile, proxy, tray, and renderer parity

**Files:**

- Modify: `src-tauri/tests/built_in_agent_switch_e2e.rs`
- Create: `tests/integration/AgentProviderSwitching.test.tsx`
- Modify: focused existing tests named in child plans

**Produces:** equivalent switch coverage for all fixed Agents and cross-surface regression evidence.

- [ ] **Step 1: Complete five built-in A→B→A cases**

For each built-in adapter, prove real selector/model change, unrelated config/MCP/comment/auth preservation, API split credential route, route health, current-state semantics, external detection/drift, rollback/recovery, and immutable event attribution. Add official Codex/Claude cases proving no client token-file access, bridge ordering/CAS/rollback/finalize, and logout/account guards.

- [ ] **Step 2: Complete legacy/takeover/Profile/proxy cases**

Prove linked legacy enters coordinator, managed-unlinked zero-writes/link flow, unmanaged legacy retains behavior/rescan, compatibility marker timing, managed takeover block, Disable takeover and switch rollback, proxy auto-start/sole compensation/guarded stop, origin batch rollback, multi-Agent Profile parent-journal all-or-rollback, and unmanaged legacy Profile round-trip.

- [ ] **Step 3: Complete native right-click/left-click cases**

Prove verified checkmark/current unknown/pending marker, fresh-token quick apply, stale/drift/target/takeover/Profile/partial main fallback, success/failure attention, unrelated Agent availability, cold pending destination once, and left-click usage popover zero configuration writes.

- [ ] **Step 4: Complete renderer integration cases**

`AgentProviderSwitching.test.tsx` covers controller before no-history empty state, Bound/Available/Effective/Switchability/Current separation, model Save/reapply, current mutation guards, affected Agents, Custom draft/wizard/edit/disconnect, sync queue, per-Agent pending, all reason/action presentation, native attention, and zero switch from navigation/sort/hide/left-click.

- [ ] **Step 5: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test built_in_agent_switch_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test profile_roundtrip --features test-hooks -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray::tests --lib
pnpm test:unit tests/integration/AgentProviderSwitching.test.tsx
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
git add src-tauri/tests/built_in_agent_switch_e2e.rs tests/integration/AgentProviderSwitching.test.tsx
git diff --cached --check
git commit -m "test: cover built-in and cross-surface agent switching"
```

## Task 5: Audit secret, path, authorization, and recovery boundaries

**Files:**

- Create: `src-tauri/tests/agent_switch_security.rs`
- Create: `scripts/check-agent-switch-secret-boundaries.mjs`
- Modify: `package.json` only if adding `check:agent-switch-secrets` as a wrapper

**Produces:** executable sentinel scan plus a code-level security review of the complete feature diff.

- [ ] **Step 1: Add multi-sentinel runtime leak tests**

Use unique local key, upstream key, OAuth token, account identifier, protected slot, raw target byte, raw SHA/digest, snapshot key, and proof-source sentinels. Exercise success, validation errors, drift, network errors, rollback, partial, recovery, sync/restore conflict, diagnostics, menu attention, and renderer serialization. Assert forbidden sentinels absent from logs/events/errors/DTOs/SQL logical exports/sync payloads/recovery metadata; encrypted blob ciphertext is not searched for plaintext equality only—decrypt verification remains backend-only.

- [ ] **Step 2: Add repository boundary scan**

The script fails on new serializable/Debug DTO fields or logging patterns named `apiKey`, `authToken`, `oauthToken`, `credentialSlot`, `credentialFingerprint`, `beforeBytes`, `snapshotKey`, `accountIdentifier`, or direct target digest within the new feature directories unless allowlisted as a protected non-serializable input wrapper. Keep allowlist small and documented in the script.

- [ ] **Step 3: Run a security diff review**

Use the repository's security-diff workflow on the complete feature range, focusing on:

- proxy auth source→audience→upstream sink;
- local/upstream credential separation and zero fallback;
- official bridge token-file boundary;
- path traversal/symlink/hardlink/TOCTOU/permissions/metadata;
- preview replay/version/CAS;
- journal crypto nonce/AAD/key cleanup and sidecar/database mismatch;
- sync/restore foreign-state execution;
- logs/DTOs/events/native menu attention.

Every validated finding receives a failing test and fix before Gate 5. Record “no finding” only with reviewed commit range and commands.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_security -- --nocapture
node scripts/check-agent-switch-secret-boundaries.mjs
git add src-tauri/tests/agent_switch_security.rs \
  scripts/check-agent-switch-secret-boundaries.mjs package.json
git diff --cached --check
git commit -m "test: audit agent switching security boundaries"
```

If `package.json` was unchanged, do not stage it.

## Task 6: Run the complete Gate 5 and write the completion audit

**Files:**

- Create: `docs/superpowers/progress/2026-07-16-agent-provider-switching-completion-audit.md`
- Modify: no production files unless a gate exposes a defect; fix defects in separate scoped commits and rerun affected plus full gates.

**Produces:** one requirement-by-requirement evidence ledger and a clean release candidate.

- [ ] **Step 1: Re-run the mandatory Custom Agent gate first**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_acceptance first_release_custom_agent_gate -- --nocapture
```

Expected: all 24 protocol/format/target cases pass. If this fails, stop completion claims even when every built-in test is green.

- [ ] **Step 2: Run all new Rust integration/security gates**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_recovery_matrix -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_account_sync -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test built_in_agent_switch_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test agent_switch_security -- --nocapture
node scripts/check-agent-switch-secret-boundaries.mjs
```

- [ ] **Step 3: Run complete Rust gates through the repository wrapper**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --tests --features test-hooks
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

- [ ] **Step 4: Run complete renderer/build gates**

```bash
pnpm typecheck
pnpm format:check
pnpm build:renderer
pnpm test:unit --exclude '.worktrees/**'
```

- [ ] **Step 5: Perform isolated macOS smoke without real user state**

Use a dedicated temporary HOME/config root and mock Providers. Confirm the status item appears, left click opens usage-only popover, right click retains native menu, one linked ready switch quick-applies, one confirmation opens main controller, Custom wizard resumes a saved draft, A→B→A works through a test config target, and quitting cleans the test proxy. Capture only redacted screenshots/logs. Do not point the build at `~/.codex`, `~/.claude`, or other real files.

- [ ] **Step 6: Write the completion audit**

The audit must contain one row for every:

- approved design goal/non-goal/product invariant;
- state/health/reason/action value;
- five built-in adapter contract;
- official bridge/legacy/Profile/takeover/proxy/menu requirement;
- Custom draft/path/format/multi-target/credential/Disconnect requirement;
- all 16 first-release Custom workflow steps and all 24 matrix cases;
- sync/restore/account/source-epoch requirement;
- security requirement;
- child-plan gate and full command above.

Each row names authoritative test/command/runtime evidence and its current result. “Implemented,” “covered elsewhere,” or a code pointer without executed evidence is insufficient.

- [ ] **Step 7: Inspect Git scope and working tree**

```bash
git status --short
git log --oneline --decorate --reverse main..HEAD
git diff --stat main...HEAD
git diff --check main...HEAD
git diff --name-only main...HEAD
```

Expected: only deliberate Agent switching docs/code/tests plus required lockfile changes; no `.pnpm-store/`, real configs, credentials, recovery blobs, logs, foreign worktree edits, or unreviewed menu-popover/root-WIP diff.

- [ ] **Step 8: Commit the evidence ledger**

```bash
git add docs/superpowers/progress/2026-07-16-agent-provider-switching-completion-audit.md
git diff --cached --check
git commit -m "docs: audit agent provider switching completion"
```

- [ ] **Step 9: Final completion decision**

Mark the first release complete only if every audit row is proven, the Custom gate is green, all full commands pass, isolated smoke passes, and the worktree is clean. Otherwise keep the program active and fix the unmet requirement; do not redefine the release around the passing subset.
