# Current Task State

Last checkpoint: 2026-07-12 09:46 Asia/Singapore

Branch: `codex/usage-dashboard-backend`

Worktree: `/Users/max/LLM Usage Bar/.worktrees/codex-usage-dashboard-backend`

Implementation HEAD: `67654d44` (`fix(ci): satisfy backend clippy gate`)

Latest pushed checkpoint before this status update: `360d4ddc` (`docs: checkpoint PR CI remediation`)

## Goal

Complete `docs/superpowers/plans/2026-07-11-usage-dashboard-backend-implementation.md`, including the newly confirmed requirement that LLM Usage Bar must coexist with the original CC Switch, whose production database supports schema v11.

## Stage status

- Task 1 — v13 domain and v12 to v13 migration: complete, reviewed, committed.
- Task 2 — Provider/event/quota persistence DAO: complete, reviewed, committed.
- Task 3 — static one-Provider route: complete and independently approved, including real local-503/zero-upstream-hit acceptance with reachable legacy candidates.
- Task 4 — transactional ingestion and trusted cost capture: complete, independently re-reviewed and approved.
- Task 5 — quota scheduler and bound Claude/Codex Session import: complete and independently approved.
- Coexistence safety — distinct app identity/data path from original CC Switch: complete and independently approved. Optional explicit snapshot import is not implemented.
- Task 6 — aggregation and nine Tauri commands: complete, independently approved, full Rust gate passed.
- Task 7 — React dashboard/configuration surface: complete and independently reviewed, including explicit Session-source ownership and moving live ranges.
- Task 8 — legacy main-path exit, real proxy E2E/runbook, desktop shell/live refresh/tests and all whole-branch review fixes: complete and independently reviewed.
- Final gate — Rust 1.95, all frontend tests, TypeScript, renderer build, static request-path scan, real proxy E2E and diff check: complete.
- PR #6 CI follow-up — complete. Initial backend Clippy failures were fixed without lint suppression, independently reviewed, pushed, and verified by a fully green remediation run.
- Dependabot PR #4/#5 follow-up — failures diagnosed but intentionally not fixed in this task. Full evidence and repair handoff: `docs/task-state/2026-07-12-dependabot-ci-failures-handoff.md`.

## Completed commits in this implementation

- `49c2f67b` docs: add usage dashboard backend execution plan
- `55814021` feat(db): add provider-aware usage schema
- `1fcaf35d` fix(db): enforce append-only quota snapshots
- `ac70ec35` feat(usage): persist providers events and quotas
- `18e146fe` fix(usage): enforce provider-safe event matching
- `f323ea75` feat(proxy): enforce static provider routes
- `7fafece4` feat(usage): ingest trusted request costs
- `1c61ce6b` docs: track usage dashboard implementation progress
- `73ccb294` fix(usage): enforce route and ingestion trust
- `d315978b` docs: checkpoint usage dashboard implementation
- `fc57e960` feat(usage): collect quotas and bound sessions
- `e931e7f9` feat(app): isolate dashboard from cc switch
- `b37f947c` docs: checkpoint coexistence and quota stages
- `49907148` fix(usage): enforce quota and session ownership
- `13b203a0` docs: checkpoint task 5 review fixes
- `7fb6a6df` fix(app): complete cc switch isolation
- `2481fc10` docs: checkpoint coexistence review fixes
- `ca38e0ef` fix(app): reject legacy data aliases
- `f50659c9` test(app): expect isolated proxy port
- `89f24a8e` test(app): isolate fixtures from cc switch data
- `42884072` test(app): expect isolated takeover port
- `fd8f7af6` feat(usage): expose product dashboard API
- `9d7ea3f8` docs: checkpoint task 6 backend API
- `958a1cf6` fix(usage): pin session sync ownership
- `2194c36d` fix(usage): harden dashboard consistency
- `f9063ff8` feat(ui): add provider-aware usage dashboard
- `ee93e413` feat(ui): make usage dashboard the main path
- `15e44174` test(usage): add real proxy dashboard acceptance
- `9575ab78` docs: add usage dashboard acceptance runbook
- `66cff322` fix(ui): complete dashboard diagnostics and localization
- `4fa50dea` test(usage): harden proxy dashboard acceptance
- `0e27e91c` refactor(proxy): remove legacy selection from request path
- `44ae009a` fix(ui): restore dashboard shell and live cache updates
- `759ca832` docs: checkpoint dashboard acceptance stages
- `bcc63b52` fix(usage): honor edited migrated routes
- `d0fd6f33` fix(proxy): enforce one upstream attempt
- `5f23ada6` docs: checkpoint whole-branch review fixes
- `22f85516` fix(usage): advance live dashboard ranges
- `1a04f5b7` fix(usage): expose and serialize session ownership
- `ec389d86` fix(usage): preserve session binding invariants
- `67654d44` fix(ci): satisfy backend clippy gate
- `360d4ddc` docs: checkpoint PR CI remediation

## Key decisions

1. Route binding is the per-request SSOT. The request path never reads current Provider, failover queue, or circuit-breaker fallback and never tries a second Provider.
2. `BoundProvider` preserves both identities: global `usage_provider_id` for v13 events and legacy runtime `Provider` ID for existing adapters/compatibility logs.
3. Direct route configs normalize public `baseUrl` to adapter `base_url`, and route selection validates URL plus credentials before forwarding; incomplete routes become local 503.
4. Explicit invalid upstream cost is a diagnostic ingestion failure. It does not alter the successful response and is not downgraded to estimated/unavailable.
5. Transformed SSE is tee-inspected before protocol conversion so exact upstream IDs and cost fields survive transformations.
6. Usage event/link/legacy compatibility log writes are one SQLite transaction. Only `event_id` conflict is idempotently ignored.
7. Session import requires an explicit source binding and advances a file offset only after all yielded records have ingested successfully.
8. Original CC Switch data must remain untouched. LLM Usage Bar will use a distinct product/bundle identity and default directory (`~/.llm-usage-bar`). Do not auto-upgrade `~/.cc-switch/cc-switch.db`.
9. Any future import from original CC Switch must be an explicit user action using a read-only SQLite Backup snapshot, never `fs::copy` of a live/WAL database. The two apps must not simultaneously take over the same CLI live configuration.
10. LLM Usage Bar registers no custom deep-link scheme in this milestone. A temporary `llmusagebar://` registration was rejected because the retained runtime parser still accepted only `ccswitch://`; claiming a nonfunctional scheme was worse than registering none.
11. Machine-global Claude/Codex quota credentials may belong to only one enabled subscription Provider per local quota source. Provider Session sync uses only explicit source bindings and aggregates every source bound to that Provider; it never guesses from product or legacy metadata.
12. Subscription dashboard DTOs expose a redacted successful quota snapshot separately from `quota_fetch_state`, so a first-attempt failure remains visible even when no successful snapshot exists.
13. On Unix, original-data override protection compares filesystem object identity (`dev` + `ino`) in addition to lexical/canonical paths; this closes macOS case-alias and symlink bypasses.
14. Quota fetch-state writes are monotonic by `attempted_at`; equal-time success may replace failure, but failure cannot replace equal-time success. Dashboard reads the latest snapshot/state under one database lock.
15. Historical usage remains grouped by immutable `UsageEvent.product_group_id` even if the Provider is later reclassified. Provider and product cost sums use checked Decimal addition and return an error on overflow.
16. The real proxy acceptance must seed reachable legacy current/failover candidates in its route-less fixture; an empty database cannot prove that fallback is unreachable.
17. Integration tests use the existing `ProxyService` boundary. Production `ProxyServer`/`ProxyConfig` APIs are not widened solely for test access.
18. Legacy feature removal does not include the desktop window shell: drag regions and optional native-like controls remain required while only switching/failover/MCP/Skills/OpenClaw/cloud business entry points leave the render tree.
19. A migrated Provider keeps its legacy runtime ID and metadata only as a compatibility envelope; v13 `route_config` is always the routing SSOT.
20. The v13 request path disables reactive media/thinking/budget resends. Preventive transformations remain, but one client request makes exactly one upstream attempt.
21. Claude/Codex Session ownership is explicit in the existing Provider save DTO/view/UI. Provider and ownership changes commit in one SQLite transaction; no tenth Tauri command is added.
22. In-flight bound Session sync and binding mutations are serialized with a dedicated operation mutex. The mutex spans file IO but never holds the SQLite connection across IO, so offset and ownership have one linear order.
23. Dashboard live ranges use an exclusive `current second + 1` end and advance on `usage-log-recorded` plus a 30-second tick; fixed custom ranges do not move.
24. CI warnings are treated as required release gates. The response logging arguments form an owned `UsageLogParams` payload rather than suppressing `clippy::too_many_arguments`; quota collectors receive the existing Provider reference directly rather than an immediately dereferenced double reference.

## Failures and review findings already handled

- A stale `tauri dev` process held the Cargo artifact lock; it was stopped. Do not restart the desktop app before path isolation is complete.
- Initial static route implementation returned legacy Providers before validating config and lost the global Provider ID. Both were fixed.
- Initial ingestion used broad `ON CONFLICT DO NOTHING`; a regression test proved non-event conflicts were silently swallowed. It now uses `ON CONFLICT(event_id) DO NOTHING`.
- Initial invalid explicit cost handling reported an error but continued as estimated/unavailable. It now skips ingestion.
- Initial transformed SSE collection ran after conversion and could lose raw cost/rewrite IDs. Raw tee capture now preserves both.
- Task 5 RED tests initially failed because quota/session APIs were absent, then two test-only `lock_conn!` uses failed to compile in `fn -> ()`; test locks were corrected.
- Task 5 review found that `sync_provider(A)` could follow source binding B and that Claude session IDs used a different stable-ID column than proxy events. Provider equality is now enforced and raw Claude message IDs populate `upstream_correlation_id`.
- Task 5 re-review found unsupported/normalization failures did not persist `last_attempt_at`, machine-global quota sources could be attributed to several Providers, and `sync_provider` guessed a single source. Commit `49907148` persists every attempt, enforces local quota ownership, and treats explicit bindings as the sole source of truth.
- The first coexistence draft registered `llmusagebar://`, but the legacy runtime handler only parsed `ccswitch://`. The final isolation commit registers no scheme and therefore cannot steal or break original CC Switch deep links.
- Task 3 and Task 4 independent re-review approved the implemented route/ingestion fixes. Task 8 later supplied and independently approved the true proxy 503/zero-hit proof.
- Coexistence review found residual `CC Switch` Windows/tray labels, frontend `~/.cc-switch` fallbacks, and no real symlink regression test. These are the next isolation fixes.
- Those coexistence findings were fixed in `7fb6a6df`: Windows/tray branding and frontend fallbacks now use LLM Usage Bar, and an actual filesystem symlink to a fake legacy directory is rejected by the same validation boundary.
- The first repeat check stopped at `cargo fmt --check` because the new tests needed formatting; formatting was applied and the complete targeted gate then passed.
- Coexistence re-review found that a macOS case alias such as `~/.CC-SWITCH` can resolve to the original directory while retaining different canonical spelling. `ca38e0ef` now compares filesystem object identity; the reviewer approved the fix.
- The first Task 6 full Rust run exposed six stale 15721 assertions after the deliberate proxy-port isolation to 15722. Targeted assertions were updated; all affected tests passed.
- The second full run exposed integration fixtures still writing/cleaning current app data under `.cc-switch`; the first cleanup-only attempt was insufficient because test paths themselves were stale. All current-app fixtures now use `.llm-usage-bar` while coexistence tests retain `.cc-switch` only as the protected legacy path.
- A later Provider integration assertion still expected takeover port 15721 and poisoned six sibling tests after its failure. Updating that one expected output to 15722 restored all 33 Provider integration tests.
- Task 5 re-review found a binding-change TOCTOU window in `sync_provider(A)`. `958a1cf6` pins the requested Provider through the second binding check; A→B changes now warn without scanning or importing. Re-review approved.
- Task 6 review found Decimal panic risk, current-Provider product regrouping of historical events, inconsistent quota pair reads, non-monotonic concurrent quota completion, and incomplete result redaction tests. `2194c36d` fixes all five areas; re-review approved.
- Task 7 review found interval-zero corruption, ignored base-URL-only edits, invisible query/mutation failures, incomplete subscription summaries, incomplete localization and weak date-range assertions. `66cff322` fixes all Important items and exact range coverage; re-review left two Minor diagnostics now included in Task 8 frontend work.
- Task 8 frontend review found that the initial main-path reduction also removed the desktop drag/window-control shell, did not invalidate v13 dashboard/event queries on `usage-log-recorded`, and used a self-proving mocked App test while the old integration suite stayed red. The repair keeps only the minimal shell and rewrites tests for the new product contract.
- Task 8 backend review found test-only public API expansion and a route-less empty database that could not detect legacy fallback regression. `4fa50dea` uses existing `ProxyService`, seeds reachable legacy candidates, widens the async query window and asserts the full exact aggregate; independent re-review approved.
- Whole-branch review found six Important integration gaps: no public Session-source binding path, stale legacy route config authority, frozen live range, scan/rebind ownership race, URL-embedded secret exposure and reactive second upstream attempts.
- `bcc63b52` fixes the stale route authority and strips URL userinfo/query/fragment from public DTOs while recognizing nested migrated credentials. The migrated-route regression was RED on the old path and GREEN after the fix.
- `d0fd6f33` disables reactive resends on the v13 main path. The real proxy probe first observed two upstream hits for one request, then passed with exactly one hit.
- Whole-branch review also found no public Session binding path and a scan/rebind ownership race. `1a04f5b7` adds atomic Provider/binding save, four-language UI and a dedicated sync/binding operation guard; transaction rollback and both Claude/Codex bound-entry tests pass.
- Re-review found one Minor omission invariant: an internal caller could omit bindings while removing `session_log`. `ec389d86` first reproduced the invalid retained binding, then rejects the update before UPSERT and preserves the original Provider/binding. Final re-review found no remaining findings.
- The first PR #6 backend CI run failed before tests because the local final gate had not included the workflow's `cargo clippy ... -D warnings` command. It exposed `spawn_log_usage` with 9 parameters and a needless `&provider` borrow. The failures were reproduced locally, fixed in `67654d44` without lint suppression, and independently reviewed as ownership- and behavior-safe.
- GitHub Actions remediation run `29162690278` then passed Backend Checks, Frontend Checks and PR labelling. Backend Clippy completed before the full Rust test step, closing the original CI failure rather than bypassing it.

## Latest stage verification

Using repository-pinned Rust 1.95 temporary toolchain environment:

- `cargo fmt --check`: passed.
- `proxy::provider_router::tests`: 12 passed.
- `usage::ingestion`: 6 passed.
- `response_processor`: 13 passed.
- `usage::quota`: 8 passed after review fixes.
- `usage::session`: 4 passed after review fixes.
- Final Task 5 gate: quota 8, Session service 4, underlying Session parser/service group 36 passed.
- Coexistence identity integration test: 1 passed.
- Config-path direct and actual-symlink override guards: 1 + 1 passed.
- macOS case-alias override guard: 1 passed; independent coexistence review approved.
- Task 6 dashboard aggregation tests: 6 passed.
- Quota DAO monotonic/consistent status tests: 5 passed; quota service tests: 9 passed.
- Task 6 nine-command integration tests: 2 passed.
- Final full Rust 1.95 gate: library 1857 passed / 2 ignored; all integration test binaries passed, including nine-command and real proxy E2E tests.
- `git diff --check`: passed.
- Task 7 targeted frontend tests after Important fixes: 15 passed; `tsc --noEmit` and `vite build` passed.
- Real mock-upstream proxy acceptance after hardening: 1 passed; exact aggregate cost `0.42012` and route-less legacy-candidate 503/zero-hit behavior verified.
- Provider router after removing unused legacy selection: 11 passed.
- Required request-path search for `get_effective_current_provider|get_failover_queue|select_providers`: zero matches.
- Final frontend gate: 72 test files / 433 tests passed; `tsc --noEmit` and `vite build` passed.
- Whole-branch review and all six Important follow-up reviews: approved; final review has no Critical, Important or Minor findings.
- No desktop/Tauri application was launched; tests used isolated/in-memory databases.
- PR #6 Clippy remediation: exact workflow command `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passed; response processor 13/13 and quota 9/9 targeted tests passed; the complete Rust gate again passed with 1857/2 plus all integration binaries.
- PR #6 remote remediation gate: Backend Checks passed in 13m34s, Frontend Checks passed in 2m43s, and PR labelling passed in 8s on GitHub Actions run `29162690278`.

## Real-data safety observation

- `/Users/max/.cc-switch/cc-switch.db` currently reports `PRAGMA user_version = 11`.
- The original `cc-switch` process (PID 31434 at this checkpoint) currently holds the database open and is actively updating it. Its current observed SHA-256 is `c105618cb3d52512443584a4cef1536cb522edd078b5e09cd8af03c5b457b49c`, mtime `1783785082`, size `8998912`; the version remains 11.
- The hash changed since the earlier checkpoint because the original app is live, so long-window hash equality is not a valid coexistence proof. Isolation is instead verified through distinct identity/default paths, override rejection (including symlink), and tests that use fake/in-memory homes without launching this app.
- The original installed app and this branch previously shared `com.ccswitch.desktop` and `~/.cc-switch`.
- A prior v13 backup artifact exists, proving the collision is not theoretical.
- Never run `tauri dev` or a development build with the real user home during this plan; use isolated test homes and in-memory databases.

## Immediate next actions

1. No required implementation or PR #6 CI remediation work remains for this plan.
2. Handle Dependabot PR #4 and #5 in a separate task using `docs/task-state/2026-07-12-dependabot-ci-failures-handoff.md`.
3. Keep optional original-data import deferred unless a future explicit read-only snapshot workflow is requested.
