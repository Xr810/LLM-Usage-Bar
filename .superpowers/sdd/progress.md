# Menu Bar Usage Popover — SDD Progress

This active ledger belongs to branch `codex/menu-bar-usage-popover` in the
isolated worktree `.worktrees/menu-bar-usage-popover`. The prior Agent-Centric
ledger is retained below as historical evidence.

## Active Baseline

- Plan: `docs/superpowers/plans/2026-07-15-menu-bar-usage-popover-implementation.md`
- Base commit: `b42b4b8e` (`docs: add menu bar usage popover implementation plan`)
- Preflight correction: `b9a51b32` (`docs: align popover plan with production dependency injection`)
- Frontend focused baseline: `src/i18n/dashboardCopy.test.ts` passes 5/5.
- Rust focused baseline: `database::tests::migration_v16_to_v17` passes 6/6.
- The root checkout's user-owned Settings/dashboard edits and `.pnpm-store/`
  remain outside this worktree.
- Worktree pnpm commands use `--config.verify-deps-before-run=false` with the
  root checkout's existing dependency store; no network install is required.

## Active Task Ledger

| Task | State | Commit | Verification |
| --- | --- | --- | --- |
| 1. Schema v18 and daily budget persistence | in progress | — | baseline passed; RED/GREEN pending |
| 2. Usage severity policy | pending | — | — |
| 3. Usage snapshot projection | pending | — | — |
| 4. Refresh orchestration and stale snapshot | pending | — | — |
| 5. Real menu-bar dot assets | pending | — | — |
| 6. macOS tray and popover window contract | pending | — | — |
| 7. Popover renderer shell and payload | pending | — | — |
| 8. Compact usage popover UI | pending | — | — |
| 9. Daily budget settings | pending | — | — |
| 10. Full verification and visual QA | pending | — | — |

## Active Review Notes

- A task is complete only after focused verification and independent review.
- Preserve the last successful snapshot on refresh failure and never expose raw
  provider configuration or credentials in renderer-safe payloads.
- Subscription severity is green above 50%, yellow from 20% through 50%, and
  red below 20%; metered severity is green below 50% of budget, yellow from 50%
  through below 80%, and red at or above 80%.

---

# Agent-Centric Usage Modules — SDD Progress

This ledger belongs to branch `codex/agent-centric-usage-modules` in the isolated
worktree `.worktrees/agent-centric-usage-modules`.

## Baseline

- Commit: `e653d977` (`docs: mark dynamic dashboard plan complete`)
- Frontend: 90 test files / 511 tests passing.
- Rust integration baseline: three pre-existing fixtures fail to compile because
  `UsageProviderInput` initializers omit `dashboard_module_id`.
- Rust library baseline: 1961 passed, 2 ignored, 15 sandbox-only listener failures
  (`Operation not permitted`); final listener tests must run outside the sandbox.
- Root checkout changes in Settings close-control files are user-owned and excluded.

## Task Ledger

| Task | State | Commit | Verification |
| --- | --- | --- | --- |
| 0. Persist execution plan | completed | plan commit | two independent reviews approved; `git diff --check` passes |
| 1. v16 schema and conservative history migration | completed | this task commit | focused 5/5; schema 5/5; integration 3/3; library 1981 passed / 2 ignored; both reviews approved |
| 2. Agent module and binding persistence | completed | this task commit | focused 7/12/15; integration 4/4; library 1997 passed / 2 ignored; both reviews approved |
| 3. Protected credential store and atomic key lifecycle | completed | this task commit | credentials 72/72; backup 30/30; library 2057 passed / 2 ignored; release check and Clippy pass; reviews approved |
| 4. Credential-routed proxy and frozen event attribution | completed | this task commit | credentials 84/84; response guard 35/35; proxy E2E 16/16; library 2160 passed / 2 ignored; Clippy and both reviews approved |
| 5. Trusted session attribution and Agent-safe dedup | completed | this task commit | session 9/9; ingestion 18/18; event DAO 11/11; session services 43/43; library 2176 passed / 2 ignored; Clippy and both reviews approved |
| 6. Agent dashboard, events, diagnostics, commands | completed | this task commit | dashboard 11/11; commands 9/9; integration 4/4; quota 12/12; proxy E2E 16/16; library 2195 passed / 2 ignored; both reviews approved |
| 7. Agent-centric frontend and Settings | completed | this task commit | frontend unit 92/509; typecheck, renderer build, format, and both reviews approved |
| 8. Integration, isolation review, and full acceptance | completed | this task commit | frontend 92/513; Rust library 2201 passed / 2 ignored; integration 141/141; format, typecheck, build, Clippy, and reviews approved |

## Review Notes

- No task is complete until both spec-compliance and code-quality review findings
  are resolved and the task's focused verification is fresh.
- Never record raw binding keys, fingerprints, or secret prefixes in this file.

## Task 1 Evidence

- RED: the first focused compile failed only because
  `crate::usage::agent_module_migration` did not exist.
- GREEN: `database::tests::migration_v15_to_v16` runs 5 tests and all pass;
  `database::tests::schema_migration` runs 5 tests and all pass.
- Both formerly broken usage-dashboard integration targets now execute successfully
  (3 tests total), including the localhost proxy e2e.
- Rust format, `git diff --check`, and Clippy pass for the changed code; Clippy's
  pre-existing watcher dead-code warnings were excluded from the quality gate.
- Independent spec-compliance and code-quality reviews approved the final Task 1
  implementation.
- Final library gate ran outside the sandbox with one test thread to avoid existing
  process-wide HOME races: 1981 passed, 2 ignored, 0 failed.

## Task 2 Evidence

- RED: the first focused compile failed only on the planned missing
  `AgentModuleInput`, `AgentProviderBindingInput`, Agent DAO methods, and binding
  DAO methods.
- GREEN: Agent DAO tests pass 7/7, binding DAO tests pass 12/12, Provider DAO
  tests pass 15/15, and domain/compatibility command tests pass.
- The active Provider DTO and save path no longer read, project, or update the v15
  single-membership column; `UsageProviderView.bindings` is authoritative and
  Provider updates preserve every binding.
- Managed-account, Codex OAuth, Gemini OAuth, and conflicting auth metadata all
  fail closed. Private binding records containing legacy config, slots, or
  fingerprints cannot be formatted with `Debug`.
- Both independent reviews approved after all findings were resolved. Rust format,
  `git diff --check`, and Clippy with the pre-existing watcher dead-code warning
  class excluded all pass.
- The command integration target passes 3/3 and the localhost proxy e2e passes
  1/1. Final library verification ran outside the sandbox with one test thread:
  1997 passed, 2 ignored, 0 failed.

## Task 3 Evidence

- RED: the first focused credential lifecycle compile failed on `E0583` because
  the planned `credentials::service` implementation did not exist; the failing
  behavior test requires a successful first set followed by a second set returning
  `credential_conflict` rather than silently replacing the active generation.
- Protected values live only in the platform credential store; SQLite contains a
  domain-separated SHA-256 fingerprint, opaque slot, version, and crash journal.
  Secret wrappers are non-serializable, non-cloneable, zeroized, and redact Debug.
- Set/replace/clear/delete use ordered journal generations, compare-and-swap, global
  active-slot checks, cancellation-safe in-process and cross-process lifecycle
  locks, startup reconciliation, and idempotent cleanup.
- SQL import, WebDAV/S3 sync, and binary restore cannot orphan local credential
  slots or import foreign deletion intent. Sync preserves local credential columns
  and exports no journal rows.
- Security review reproduced route-retargeting through both direct Provider edits
  and delayed SQLite triggers. Protected snapshots now pin binding/provider
  enablement, Agent archive state, route/auth configuration, and legacy auth
  context. Imported schemas must match the canonical object allowlist, and every
  trigger including reserved `sqlite_*` names must match canonical SQL exactly.
- Focused verification passes 72 credential tests and 30 backup tests. Rust format,
  `git diff --check`, release `cargo check`, and Clippy pass. Final library
  verification ran outside the sandbox: 2057 passed, 2 ignored, 0 failed.
- Independent spec-compliance, code-quality, and final adversarial security reviews
  approved the completed Task 3 implementation.

## Task 4 Evidence

- RED: the static-route baseline allowed an unknown request credential to select a
  configured upstream and could not attach immutable Agent ownership. The first
  strengthened proxy E2E failed on the upstream hit count/ownership assertions.
- Every supported endpoint now extracts one protocol-specific binding credential,
  rejects missing, ambiguous, unknown, disabled, archived, store-missing, or
  protocol-mismatched bindings locally, and strips all local credential locations
  before constructing the upstream request.
- The second binding lookup after protected-store verification is the pre-send
  linearization point. Route projection, Agent/Provider/binding IDs, product group,
  route protocol, pricing override, and protected upstream key are frozen together;
  asynchronous ingestion receives that frozen context and cannot observe later
  binding or Provider edits.
- Runtime routes are rebuilt from an allowlisted credential-free projection. Legacy
  auth fields, nested config/env/TOML secrets, URL userinfo/query credentials,
  redirects, reflected credential headers, and uninspectable compressed streams
  cannot reintroduce a binding key.
- Response guards quarantine semantic SSE channels by stable protocol identity,
  cover raw/percent/form/JSON normalization across chunk and event boundaries, and
  reject a completed credential before transformers, caches, usage parsing, logs,
  or client egress. State, depth, normalization fan-out, and pending bytes are
  bounded; UTF-8 BOM, CR-only SSE, EOF prefixes, parallel choices, reasoning, and
  thinking paths have regression coverage.
- Focused verification passes 84 credential tests, 35 response-processor tests,
  16 Gemini streaming tests, 14/12/16 protocol conversion tests, 14 ingestion
  tests, 16 binding extraction tests, and 21 route projection tests. Rust format,
  `git diff --check`, and Clippy pass. Final listener-capable verification ran
  outside the sandbox: proxy E2E 16/16 and library 2160 passed, 2 ignored.
- Independent spec-compliance and route/ownership reviews approved the final tree
  with no P0-P2 findings after the code-quality and Clippy findings were resolved.

## Task 5 Evidence

- RED: the provider-only baseline left Claude/Codex session events without a fixed
  Agent, accepted a Provider bound only to the wrong Agent, linked exact IDs across
  different Agents, reversed canonical ownership for session-first arrival, and
  globally collided Codex files without `session_meta`.
- Claude and Codex now carry the fixed `claude-code` and `codex` Agent IDs. Bound
  entrypoints and the ingestion transaction both require the same enabled
  Agent-Provider binding, enabled Provider, and non-archived Agent; rebinding and
  rescans cannot mutate prior event ownership.
- Cross-source matching and link insertion require identical non-null Agent and
  Provider ownership plus an exact identifier. Proxy/upstream-cost events remain
  canonical for both arrival orders without rewriting immutable events.
- Codex no-meta identities come from the same opened file handle used for parsing:
  Unix uses device/inode and Windows uses volume/file-index, hashed into an opaque
  entity cursor and event scope. Archive moves keep identity, different files with
  identical content do not collide, and path reuse receives a new identity.
- Legacy path cursors are promoted with a single SQLite transaction that upserts
  the entity key and retires the old key while preserving parser state and offsets.
  RED/GREEN coverage proves the retired path cannot poison a later replacement
  file.
- Focused verification passes session 9/9, ingestion 18/18, event DAO 11/11,
  Codex 22/22, and all session services 43/43. Rust format, `git diff --check`, and
  Clippy with `-D warnings` pass. The parallel full suite reproduced the known
  process-wide `HOME` test race; the required single-thread rerun passed 2176 with
  2 ignored and 0 failed.
- Independent spec-compliance, code-quality, and file-identity reviews approved
  the final tree with no P0-P2 findings. Windows identity code was statically
  reviewed against the repository's existing implementation because no Windows
  target is installed on this Mac.
- Task 6 carry-forward: preserve every legacy link row. Runtime duplicate
  exclusion may trust a link only when both endpoints have the same non-null Agent
  and Provider, must select canonical ownership by `source='proxy'`, and must expose
  invalid legacy links as diagnostics rather than rewriting migration history.

## Task 6 Evidence

- RED: the legacy dashboard accepted an unscoped query and combined other-Agent and
  unassigned history. Agent-scoped tests also exposed missing historical-only quota,
  incomplete multi-binding cleanup, unsafe proxy origins, and a successful-delete
  invalidation bypass before each production fix.
- Dashboard and event queries now require immutable Agent ownership. Provider cards
  are the distinct union of display-active bindings and in-range history; quota is
  attached once per Provider and `shared_account` uses protected-store-verified
  effective bindings without multiplying usage totals.
- Runtime dedup trusts only proxy-to-session links whose endpoints share the same
  non-null Agent and Provider. Cross-Agent, cross-Provider, null-Agent, reversed,
  and missing-endpoint legacy links remain counted and appear in diagnostics.
- Binding/key commands use direct `SecretString` inputs and credential-aware views.
  Custom Agent deletion attempts every protected cleanup, can resume after a failed
  archived cleanup, and emits only payload-free successful invalidations. Proxy
  setup exposes local-only safe metadata and handles wildcard/IPv6/port-zero state.
- Provider-ID singleflight is shared by manual and scheduler quota collection while
  different Providers remain independent. Usage writes retain the legacy event and
  independently debounce the new payload-free dashboard invalidation.
- Focused verification passes dashboard 11/11, commands 9/9, integration 4/4,
  event/DAO/invalidation 16/16, quota 12/12, and proxy E2E 16/16. Rust format,
  `git diff --check`, and Clippy with `-D warnings` pass. The single-thread library
  suite passes 2195 with 2 ignored and 0 failed.
- Independent query/aggregation and lifecycle/security reviews approved the final
  tree with no remaining Critical or Important findings.

## Task 7 Evidence

- RED first proved the old dashboard/events invoke contract omitted the required
  Agent ID. Subsequent focused REDs covered Agent navigation, unified projection,
  Settings bindings/keys/setup/diagnostics, Provider/Proxy cleanup, four locales,
  and stateful integration fixtures.
- Runtime navigation is now the five fixed Agents plus Custom Agents, with no
  synthetic global API tab. Dashboard/event queries isolate every Agent in both
  wire inputs and cache keys, reject mismatched ownership, and globally invalidate
  after all usage mutations and the payload-free backend event.
- One Agent page preserves historical, disabled, and unbound Provider cards while
  showing subscription and metered sections together. Provider rows deduplicate by
  ID, shared-account quota is explicit, and metered totals/events remain Agent
  scoped.
- Settings owns explicit Agent-Provider bindings, protected key Set/Replace/Clear,
  safe read-only proxy instructions, and aggregate diagnostics. Provider saves
  mutate metadata only; proxy controls remain explicit; no Agent configuration or
  legacy route binding is changed by selection.
- Raw binding keys bypass React Query caches, exist only in the transient password
  input and immediate invoke, and clear on every close/success/failure path. The
  dialog freezes its opening credential version so concurrent updates fail CAS.
- Independent review findings for prior-Agent async feedback, archived navigation,
  `not_required`, `claude-desktop`, and unbound historical MSW events were resolved
  with focused RED/GREEN coverage. No Critical or Important findings remain.
- Final verification passes focused query 9/9, Agent page 6/6, Agent Settings 4/4,
  App 4/4, and full frontend 92 files / 509 tests. Typecheck, renderer build,
  Prettier, and `git diff --check` pass; no Rust or dependency files changed.

## Task 8 Evidence

- Local-only Claude, Codex, and Gemini proxy acceptance covers two distinct Agent
  bindings where applicable. Each request maps to its exact immutable Agent owner,
  and rebind, key rotation, binding deletion, and Agent archival leave prior event
  rows unchanged.
- Unknown, disabled, cleared, archived, store-missing, and mismatched bindings are
  rejected locally before any upstream request. Public events, diagnostics, SQL
  export, application logs, captured URI/body data, and the frontend DOM/form/query/
  mutation snapshot omit every test binding value.
- Public Provider DTOs now hydrate their nested bindings from the verified binding
  service for list, save, and dashboard responses. Session-only multi-Agent
  bindings remain non-shared because they are not effective proxy routes.
- `canClearCredential` distinguishes an unavailable but removable protected item
  from an unavailable binding with nothing to clear. The UI exposes only the valid
  Clear action, and a failed compare-and-swap refreshes root queries while preserving
  the original error.
- Published setup URLs now work for Claude, Codex, and Gemini namespace paths.
  Claude Desktop uses only its binding-key header, while managed and unsupported
  auth modes retain status metadata without publishing unusable direct-key setup.
- Persisted Agent selection survives initial query loading. Metered shared-account
  labels, stateful fixtures, Provider counts, and compatibility-manifest locations
  match the production contract.
- Two independent Task 8 reviews approved after the Gemini exact-owner assertion,
  Claude public-snapshot coverage, Portal-aware frontend snapshot, verified Provider
  DTO hydration, setup-path, direct-auth capability, selection, shared-account, and
  unavailable-credential findings were resolved.
- Final frontend verification passes 92 files / 513 tests, typecheck, renderer build,
  and formatting. Rust verification passes 2201 library tests with 2 ignored plus
  141/141 integration tests; the local proxy target passes 20/20, and format plus
  all-target Clippy with `-D warnings` pass.
- The root checkout's user-owned Settings edits and `.pnpm-store/` remain unchanged
  and outside this branch.
