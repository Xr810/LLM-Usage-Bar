# Custom Agent Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user define a safe declarative adapter for a Custom Agent, bind protocol-compatible Providers, and perform the same verified A→B→A switching, recovery, attribution, and guarded Disconnect lifecycle as a built-in Agent.

**Architecture:** Incomplete wizard state is an installation-local draft separate from the validated adapter. A platform-gated path policy opens targets with descriptor/no-follow checks; strict JSON/TOML/YAML/dotenv editors update only declared logical mappings and return complete in-memory outputs to the existing coordinator. Custom API routes use an immutable opaque Agent audience, while legacy-combined Providers require an explicit journaled split-credential upgrade before switching.

**Tech Stack:** Rust 2021, serde_json, toml_edit, strict YAML/dotenv parsers, macOS descriptor-safe file I/O, protected credentials, v19 switch coordinator and dynamic Axum routes.

**Depends on:** Gate 2 in [`2026-07-16-agent-provider-switching-built-in-adapters.md`](2026-07-16-agent-provider-switching-built-in-adapters.md).

---

## Custom Adapter Invariants

- Cancelling before initial save creates no Agent, draft, route, ownership, or file.
- Initial Save draft atomically creates Custom Agent + permanent no-current switch state + create draft. It performs zero file writes and exposes no proxy-usable route.
- A validated adapter remains authoritative while a version-bound edit draft exists. Discarding edits changes no live bytes, ownership, route, or current state.
- Path strings and editor mappings are data only. There is no script, template evaluation, environment interpolation, command substitution, or relative traversal.
- Every accepted target maps `base_url`, `local_api_key`, and `default_model` at least once across the adapter. Optional Provider identity/upstream-model mappings do not replace those required roles.
- Two mappings cannot assign different logical values to the same field. Every occurrence of one logical value must verify.
- Missing-target creation requires existing parent selection, stored creation policy, redacted preview, and explicit confirmation.
- Existing files remain user-owned regular files with preserved supported metadata. New files are user-only. Symlinks, hardlinks, special files, unsupported ACL/flags, and denied roots fail before write.
- Custom Agent subscriptions are blocked unless a dedicated bridge explicitly supports that exact Agent and flow. First release Custom Agents switch API Providers only.
- Custom Agent usage comes through its proxy route; no generic session-log parser is created.
- A current/uncertain `legacy_combined` binding cannot be upgraded. Disconnect/switch away and read-only rescan are mandatory first.
- Backend validation is authoritative. Renderer path checks or form validation never grant switchability.

## File Responsibility Map

### Create

- `src-tauri/src/agent_switch/custom/{mod.rs,domain.rs,service.rs,path_policy.rs,adapter.rs,credential_upgrade.rs}`
- `src-tauri/src/agent_switch/custom/formats/{mod.rs,json.rs,toml.rs,yaml.rs,dotenv.rs}`
- `src-tauri/src/commands/custom_agent_adapter.rs`
- `src-tauri/tests/custom_agent_switch_e2e.rs`

### Modify

- `src-tauri/src/agent_switch/{mod.rs,service.rs,adapter.rs,route_registry.rs,recovery.rs,guard.rs}`
- `src-tauri/src/database/dao/{agent_modules.rs,agent_provider_bindings.rs,custom_agent_adapters.rs,live_target_ownership.rs}`
- `src-tauri/src/credentials/{service.rs,mod.rs}`
- `src-tauri/src/proxy/{server.rs,handlers.rs,binding_auth.rs}`
- `src-tauri/src/commands/{mod.rs,usage_dashboard.rs}`
- `src-tauri/src/{store.rs,lib.rs}`

## Shared Declarative Contract

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomAdapterFormat {
    Json,
    Toml,
    Yaml,
    Dotenv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterLogicalValue {
    BaseUrl,
    LocalApiKey,
    DefaultModel,
    ProviderAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterMappingInput {
    pub target_id: String,
    pub logical_value: AdapterLogicalValue,
    pub literal_segments: Vec<String>,
}
```

`literal_segments` are exact object/table keys or one dotenv variable name. No segment is parsed as JSONPath, TOML dotted syntax, array index, glob, regex, expression, or shell token.

## Task 1: Implement atomic Custom Agent draft lifecycle and commands

**Files:**

- Create: `src-tauri/src/agent_switch/custom/{mod.rs,domain.rs,service.rs}`
- Create: `src-tauri/src/commands/custom_agent_adapter.rs`
- Modify: `src-tauri/src/database/dao/{agent_modules.rs,custom_agent_adapters.rs}`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: all created/modified modules

**Produces:** create/edit drafts, save/resume/discard, validation command shell, version compare-and-swap, and zero live writes.

- [ ] **Step 1: Write failing create/edit draft tests**

Cover:

- cancel before save calls no command and leaves DB empty (command integration fixture proves no implicit creation);
- initial save atomically inserts Custom Agent, `setup_required` state, and create draft;
- transaction failure leaves none of the three rows;
- resume returns exact step, partial non-secret form data, validation messages, and draft version;
- discard create draft removes only draft and keeps Agent setup-required;
- validated adapter edit creates one draft bound to exact `base_adapter_version`;
- discarding edit leaves validated adapter/current/ownership bytes untouched;
- stale draft/base adapter/state version rejects save/promotion;
- fixed Agent cannot acquire a Custom draft; existing Custom Agent without adapter migrates to setup-required;
- draft DTO serialization contains no file bytes, normalized digest, credential, fingerprint, slot, or recovery identity.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_draft --lib -- --nocapture
```

- [ ] **Step 3: Implement service and command DTOs**

Expose:

```text
get_custom_agent_adapter_draft
save_custom_agent_adapter_draft
discard_custom_agent_adapter_draft
validate_custom_agent_adapter_draft
promote_custom_agent_adapter_draft
```

All commands accept one `{ input }` object with `deny_unknown_fields`. Initial save receives no caller-supplied Agent ID; backend creates the stable ID and returns it. Draft save never allocates route ownership or writes a target.

- [ ] **Step 4: Register commands and run tests**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_draft --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::custom_agent_adapter --lib
git add src-tauri/src/agent_switch/custom src-tauri/src/commands/custom_agent_adapter.rs \
  src-tauri/src/database/dao src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git diff --cached --check
git commit -m "feat: persist custom agent adapter drafts"
```

## Task 2: Enforce Custom target path and file safety

**Files:**

- Create: `src-tauri/src/agent_switch/custom/path_policy.rs`
- Modify: `src-tauri/src/agent_switch/target_io.rs`
- Modify: `src-tauri/src/agent_switch/custom/service.rs`
- Test: the same modules

**Produces:** read-only validation handles and creation plans; no format editing yet.

- [ ] **Step 1: Write failing path-policy matrix**

Use a temporary HOME and temporary App data root. Cover:

- only a leading literal `~` expansion;
- rejection of `$HOME`, `${}`, `%VAR%`, backticks, `$()`, relative paths, `..`, NUL, malformed Unicode, and ambiguous casefold duplicates;
- current-user home allowlist and denial of `.ssh`, `.gnupg`, shell startup files, Keychains, LaunchAgents, LLM Usage Bar DB/backup/sync/credential/journal/snapshot/cache roots;
- user-owned regular file requirement;
- symlink, hardlink, device, socket, FIFO, unsupported ACL/flags, wrong owner, changed parent/file ID, and >4 MiB rejection;
- duplicate canonical/case-normalized targets and collision with built-in/other Custom ownership;
- total before-image >16 MiB rejection;
- missing target allowed only with opened existing parent, exact leaf, recorded create policy, and confirmation requirement;
- new file restrictive mode and existing owner/mode preservation;
- non-macOS returns `adapter_platform_unsupported` until equivalent primitives are implemented.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_target_path_policy --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml target_io_nofollow --lib -- --nocapture
```

- [ ] **Step 3: Implement canonical target handles**

Validation returns an opaque backend handle/record containing canonical normalized identity, opened parent identity, file ID/link count/owner/mode/flags, size, expected existence, and ownership generation. Apply must revalidate through the same opened-parent/no-follow primitives; it never reopens a second unchecked path string.

- [ ] **Step 4: Implement missing-target creation policy**

The preview must identify only a user-readable redacted path and `target_creation` reason. On confirmed apply, create a temp file in the opened parent with user-only mode, fsync, atomically install only if still absent, fsync directory, and record identity. Cancel/validation performs zero writes.

- [ ] **Step 5: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_target_path_policy --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml target_io_nofollow --lib
git add src-tauri/src/agent_switch/custom/path_policy.rs \
  src-tauri/src/agent_switch/target_io.rs src-tauri/src/agent_switch/custom/service.rs
git diff --cached --check
git commit -m "feat: validate custom agent configuration targets"
```

## Task 3: Implement strict JSON, TOML, YAML, and dotenv editors

**Files:**

- Create: `src-tauri/src/agent_switch/custom/formats/{mod.rs,json.rs,toml.rs,yaml.rs,dotenv.rs}`
- Test: each format module

**Produces:** pure parse/inspect/plan/verify editors over bytes. They never write files.

- [ ] **Step 1: Write the shared editor conformance suite**

Every editor must prove required logical values, literal segments, duplicate/collision detection, same logical value at several declared targets, rejection of conflicting logical values at one field, exact intended selection, unrelated-field preservation allowed by its subset, no secret in preview/error/Debug, and deterministic reparse/verify.

- [ ] **Step 2: Write failing JSON subset tests**

Accept one standard JSON object with object-only mapped paths ending at string values. Reject comments, duplicate keys, non-object root, array traversal, non-string mapped leaf, trailing data, and unsupported encoding. Preserve unrelated object members and their semantic values.

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_json --lib -- --nocapture
```

- [ ] **Step 3: Implement JSON editor and turn green**

Use a duplicate-key-detecting parse path rather than silently accepting serde's last duplicate. Render one valid JSON document, reparse, and verify all mapped values.

- [ ] **Step 4: Write failing TOML subset tests**

Accept a `toml_edit`-round-trippable document with literal key/table traversal ending at strings. Reject duplicate definitions, array/array-of-table traversal, non-string mapped leaf, and any construct whose non-owned content cannot be preserved by the selected editor.

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_toml --lib -- --nocapture
```

- [ ] **Step 5: Implement TOML editor and turn green**

Use `toml_edit::DocumentMut`, preserve comments/decor/unknown tables, update exact literal segments, render once, reparse, and verify.

- [ ] **Step 6: Write failing YAML subset tests**

Accept one mapping-only document along mapped paths with plain/single/double-quoted scalar string leaves. Reject multiple documents, duplicate keys, anchors, aliases, tags, merge keys, sequence traversal, non-string mapped leaf, and unsupported directives.

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_yaml --lib -- --nocapture
```

- [ ] **Step 7: Implement YAML editor and turn green**

Do not use a parser that silently resolves duplicate keys or expands aliases. Preserve untouched source regions/comments for the accepted mapping subset; render only mapped scalar replacements and reparse strictly.

- [ ] **Step 8: Write failing dotenv subset tests**

Accept one assignment per mapped name with optional `export` and plain/single/double-quoted single-line string. Reject duplicate mapped names, multiline values, interpolation, command substitution, malformed quoting, NUL, and invalid variable names. Preserve comments/order/unmapped assignments.

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_dotenv --lib -- --nocapture
```

- [ ] **Step 9: Implement dotenv editor and turn green**

Keep mapped value replacement literal and select quoting that round-trips without interpolation. Verify the parsed literal, not a shell-expanded value.

- [ ] **Step 10: Run conformance suite and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_ --lib
git add src-tauri/src/agent_switch/custom/formats
git diff --cached --check
git commit -m "feat: edit declarative custom agent formats"
```

## Task 4: Validate/promote adapters and execute single/multi-file switches

**Files:**

- Create: `src-tauri/src/agent_switch/custom/adapter.rs`
- Modify: `src-tauri/src/agent_switch/custom/service.rs`
- Modify: `src-tauri/src/agent_switch/{adapter.rs,service.rs,recovery.rs,guard.rs}`
- Modify: `src-tauri/src/database/dao/{custom_agent_adapters.rs,live_target_ownership.rs}`
- Test: the same modules and `src-tauri/tests/custom_agent_switch_e2e.rs`

**Produces:** validated adapter promotion, registry publication, redacted representative preview, atomic multi-target switch, and recovery.

- [ ] **Step 1: Write failing validation/promotion/multi-target tests**

Cover required roles, protocol declaration, target/mapping uniqueness, path ownership, format validation, missing-target confirmation, zero writes on cancel, initial atomic adapter+route publication, edit base-version CAS, second-file failure rollback, external edit before rollback → partial/no overwrite, exact preservation, current state unchanged until all verify, and cleanup of terminal recovery material.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_adapter_validation --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_adapter_multi_target --lib -- --nocapture
```

- [ ] **Step 3: Implement `DeclarativeAgentConfigAdapter`**

It resolves logical values from the backend projection, loads every target through safe handles, invokes the selected pure editor, builds one `AdapterPlan`, reparses every intended output, produces a redacted diff, and delegates all live replace/verify/rollback to the coordinator.

- [ ] **Step 4: Implement initial and edit promotion**

Initial promotion validates all fields/targets and atomically publishes validated adapter + route namespace/generation + ownership claims, removes draft, and keeps state non-current until an explicit switch. An active edit that changes target/selection behavior uses a normal previewed adapter-migration transaction; otherwise require successful Disconnect. Promotion never silently runs at startup/sync.

- [ ] **Step 5: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_adapter_validation --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_adapter_multi_target --lib
git add src-tauri/src/agent_switch/custom src-tauri/src/agent_switch/service.rs \
  src-tauri/src/agent_switch/recovery.rs src-tauri/src/database/dao
git diff --cached --check
git commit -m "feat: switch custom agents transactionally"
```

## Task 5: Register dynamic Custom routes and compatibility

**Files:**

- Modify: `src-tauri/src/agent_switch/route_registry.rs`
- Modify: `src-tauri/src/proxy/{server.rs,handlers.rs,binding_auth.rs}`
- Modify: `src-tauri/src/database/dao/agent_provider_bindings.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Test: same modules and `src-tauri/tests/custom_agent_switch_e2e.rs`

**Produces:** protocol-compatible binding creation and immutable dynamic route audience.

- [ ] **Step 1: Write failing compatibility/audience tests**

Parameterize Claude/Codex/Gemini Agent-facing protocol families. Prove compatible fixed/split custom APIs can bind; protocol mismatch and every subscription without a dedicated Custom bridge are blocked; route uses `/agents/{opaque-id}/{protocol}/...`; rename leaves path stable; key under another Agent namespace fails before upstream I/O; fixed compatibility aliases still work; display name never enters path.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_protocol_compatibility --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_route_audience --lib -- --nocapture
```

- [ ] **Step 3: Replace fixed-name binding allowlist with declared protocol compatibility**

Binding save validates the Agent's registered protocol, Provider upstream protocol, and supported no-cross-translation matrix. It does not trust a frontend-compatible list. Route resolution requires namespace + protocol + credential-bound Agent agreement.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_protocol_compatibility --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_route_audience --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test custom_agent_switch_e2e route_audience -- --nocapture
git add src-tauri/src/agent_switch/route_registry.rs src-tauri/src/proxy \
  src-tauri/src/database/dao/agent_provider_bindings.rs src-tauri/src/commands/usage_dashboard.rs
git diff --cached --check
git commit -m "feat: route dynamic custom agent audiences"
```

## Task 6: Upgrade legacy-combined custom Provider credentials safely

**Files:**

- Create: `src-tauri/src/agent_switch/custom/credential_upgrade.rs`
- Modify: `src-tauri/src/credentials/service.rs`
- Modify: `src-tauri/src/agent_switch/{guard.rs,recovery.rs,service.rs}`
- Modify: `src-tauri/src/database/dao/{account_claims.rs,agent_provider_bindings.rs,usage_providers.rs}`
- Modify: `src-tauri/src/commands/custom_agent_adapter.rs`
- Test: the same modules

**Produces:** preview/apply/reconcile commands for reserve→stage→verify→publish→cleanup split upgrade.

- [ ] **Step 1: Write failing upgrade safety/recovery tests**

Cover one equal combined secret, differing binding fingerprints requiring separate Providers/accounts, current/last-verified/partial/unknown-live target blocks, generated independent `lub_*` keys, protected route verification, same-account proof, publish point, every failpoint preserving old route, startup completion/rollback, and no old-slot deletion before durable publish.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml legacy_combined_upgrade --lib -- --nocapture
```

- [ ] **Step 3: Implement preview/apply journal**

Expose `preview_legacy_credential_upgrade` and `apply_legacy_credential_upgrade`. Preview never reveals equality fingerprints or secret hints. Apply proves the binding is not selected in any managed target, stages one Provider upstream credential and per-binding independent local keys, tests route/account continuity, atomically publishes `provider_split` metadata, then cleans obsolete slots. It does not switch or rewrite an Agent file.

- [ ] **Step 4: Run and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml legacy_combined_upgrade --lib
git add src-tauri/src/agent_switch/custom/credential_upgrade.rs \
  src-tauri/src/credentials/service.rs src-tauri/src/agent_switch \
  src-tauri/src/database/dao src-tauri/src/commands/custom_agent_adapter.rs
git diff --cached --check
git commit -m "feat: split custom provider credentials safely"
```

## Task 7: Guard adapter edit, Disconnect, deletion, and complete Gate 3

**Files:**

- Modify: `src-tauri/src/agent_switch/custom/service.rs`
- Modify: `src-tauri/src/agent_switch/{service.rs,guard.rs,recovery.rs}`
- Modify: `src-tauri/src/database/dao/{agent_modules.rs,agent_provider_bindings.rs,custom_agent_adapters.rs}`
- Modify: `src-tauri/src/commands/{custom_agent_adapter.rs,usage_dashboard.rs}`
- Test: same modules and `src-tauri/tests/custom_agent_switch_e2e.rs`

- [ ] **Step 1: Write failing guarded lifecycle tests**

Prove current/uncertain Agent cannot delete, archive, unbind, disable, clear credential, or remove adapter. Disconnect requires preview, exact versions, protected snapshot, removal of only owned selected values, verification that no local key remains selected, current clear in the DB commit, route/ownership release, source gap, and recovery. Delete afterward follows existing history-safe behavior. Discarding initial draft leaves setup-required; deleting that Agent is a separate guarded action.

- [ ] **Step 2: Confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_disconnect --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_delete_guard --lib -- --nocapture
```

- [ ] **Step 3: Implement guarded Disconnect and adapter removal**

Disconnect is a coordinator transaction, not a metadata toggle. If non-owned or externally changed content prevents safe removal, return drift/partial and do not claim disconnected. Adapter removal after a proven disconnect releases local path ownership but never deletes unrelated files.

- [ ] **Step 4: Run Gate 3**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_agent_ --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml custom_format_ --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml legacy_combined_upgrade --lib
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test custom_agent_switch_e2e -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

- [ ] **Step 5: Commit and record Gate 3 evidence**

```bash
git add src-tauri/src/agent_switch src-tauri/src/database/dao \
  src-tauri/src/commands src-tauri/tests/custom_agent_switch_e2e.rs
git diff --cached --check
git commit -m "feat: guard custom agent adapter lifecycle"
```

Do not start renderer work until the complete Custom backend DTO/command contract is stable and the multi-file/credential/audience/disconnect tests are green.
