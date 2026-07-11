# Current Task State

Last checkpoint: 2026-07-11 21:20 Asia/Singapore

Branch: `codex/usage-dashboard-backend`

Worktree: `/Users/max/LLM Usage Bar/.worktrees/codex-usage-dashboard-backend`

Current HEAD: `73ccb294` (`fix(usage): enforce route and ingestion trust`)

## Goal

Complete `docs/superpowers/plans/2026-07-11-usage-dashboard-backend-implementation.md`, including the newly confirmed requirement that LLM Usage Bar must coexist with the original CC Switch, whose production database supports schema v11.

## Stage status

- Task 1 — v13 domain and v12 to v13 migration: complete, reviewed, committed.
- Task 2 — Provider/event/quota persistence DAO: complete, reviewed, committed.
- Task 3 — static one-Provider route: core implementation and review fixes complete. Real local-503/zero-upstream-hit acceptance remains in Task 8.
- Task 4 — transactional ingestion and trusted cost capture: implementation plus review fixes complete; independent re-review still required.
- Task 5 — quota scheduler and bound Claude/Codex Session import: in progress in the working tree; targeted tests pass, two binding/correlation fixes are being applied before commit.
- Coexistence safety — distinct app identity/data path from original CC Switch: in progress; must complete before any desktop launch.
- Task 6 — aggregation and nine Tauri commands: not started.
- Task 7 — React dashboard/configuration surface: not started.
- Task 8 — hide legacy main-path entry points, real mock-upstream proxy acceptance, full gate: not started.

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

## Failures and review findings already handled

- A stale `tauri dev` process held the Cargo artifact lock; it was stopped. Do not restart the desktop app before path isolation is complete.
- Initial static route implementation returned legacy Providers before validating config and lost the global Provider ID. Both were fixed.
- Initial ingestion used broad `ON CONFLICT DO NOTHING`; a regression test proved non-event conflicts were silently swallowed. It now uses `ON CONFLICT(event_id) DO NOTHING`.
- Initial invalid explicit cost handling reported an error but continued as estimated/unavailable. It now skips ingestion.
- Initial transformed SSE collection ran after conversion and could lose raw cost/rewrite IDs. Raw tee capture now preserves both.
- Task 5 RED tests initially failed because quota/session APIs were absent, then two test-only `lock_conn!` uses failed to compile in `fn -> ()`; test locks were corrected.
- Independent review still requires a re-run against `73ccb294`; Task 8 still owes the true proxy 503/zero-hit mock-upstream proof.

## Latest stage verification

Using repository-pinned Rust 1.95 temporary toolchain environment:

- `cargo fmt --check`: passed.
- `proxy::provider_router::tests`: 12 passed.
- `usage::ingestion`: 6 passed.
- `response_processor`: 13 passed.
- `usage::quota`: 6 passed.
- `usage::session`: 2 passed.
- `git diff --check`: passed.
- No desktop/Tauri application was launched; tests used isolated/in-memory databases.

## Real-data safety observation

- `/Users/max/.cc-switch/cc-switch.db` currently reports `PRAGMA user_version = 11`.
- The original installed app and this branch previously shared `com.ccswitch.desktop` and `~/.cc-switch`.
- A prior v13 backup artifact exists, proving the collision is not theoretical.
- Until coexistence isolation is committed and verified, never run `tauri dev` or the built dashboard with the real user home.

## Immediate next actions

1. Finish and commit Task 5 after Provider-target binding and raw Claude message-ID link regressions pass.
2. Complete product/bundle/data-path/updater/deep-link/proxy-port isolation and add path-safety tests.
3. Independently re-review Task 3/4 fixes and Task 5.
4. Implement Task 6 aggregation and nine Tauri commands.
5. Implement Task 7 frontend.
6. Implement Task 8 mock-upstream E2E, hidden legacy UI, acceptance documentation, and full backend/frontend gate.
