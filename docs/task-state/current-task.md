# Current Task State

Last checkpoint: 2026-07-11 23:49 Asia/Singapore

Branch: `codex/usage-dashboard-backend`

Worktree: `/Users/max/LLM Usage Bar/.worktrees/codex-usage-dashboard-backend`

Current HEAD: `49907148` (`fix(usage): enforce quota and session ownership`)

## Goal

Complete `docs/superpowers/plans/2026-07-11-usage-dashboard-backend-implementation.md`, including the newly confirmed requirement that LLM Usage Bar must coexist with the original CC Switch, whose production database supports schema v11.

## Stage status

- Task 1 — v13 domain and v12 to v13 migration: complete, reviewed, committed.
- Task 2 — Provider/event/quota persistence DAO: complete, reviewed, committed.
- Task 3 — static one-Provider route: core implementation and review fixes complete. Real local-503/zero-upstream-hit acceptance remains in Task 8.
- Task 4 — transactional ingestion and trusted cost capture: complete, independently re-reviewed and approved.
- Task 5 — quota scheduler and bound Claude/Codex Session import: complete; independent review findings fixed and targeted tests pass.
- Coexistence safety — distinct app identity/data path from original CC Switch: core isolation complete and committed. Review found three remaining surface/test gaps (Windows title/tray tooltip, frontend fallback paths, actual symlink regression). Optional explicit snapshot import is not implemented.
- Task 6 — aggregation and nine Tauri commands: in progress; RED tests and initial implementation are present but not yet stage-verified or committed.
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
- `d315978b` docs: checkpoint usage dashboard implementation
- `fc57e960` feat(usage): collect quotas and bound sessions
- `e931e7f9` feat(app): isolate dashboard from cc switch
- `b37f947c` docs: checkpoint coexistence and quota stages
- `49907148` fix(usage): enforce quota and session ownership

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
- Task 3 and Task 4 independent re-review approved the implemented route/ingestion fixes. Task 8 still owes the true proxy 503/zero-hit mock-upstream proof.
- Coexistence review found residual `CC Switch` Windows/tray labels, frontend `~/.cc-switch` fallbacks, and no real symlink regression test. These are the next isolation fixes.

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
- Config-path and Store legacy-override guards: 1 + 1 passed.
- `git diff --check`: passed.
- No desktop/Tauri application was launched; tests used isolated/in-memory databases.

## Real-data safety observation

- `/Users/max/.cc-switch/cc-switch.db` currently reports `PRAGMA user_version = 11`.
- Before and after coexistence tests its SHA-256 stayed `94f3b946ad912c224bb2f88085dc2d93c059b0e7d8f6ac6aa58fa0869b45379e`; mtime `1783783942` and size `8941568` also stayed unchanged.
- The original installed app and this branch previously shared `com.ccswitch.desktop` and `~/.cc-switch`.
- A prior v13 backup artifact exists, proving the collision is not theoretical.
- Until coexistence isolation is committed and verified, never run `tauri dev` or the built dashboard with the real user home.

## Immediate next actions

1. Fix and verify the three remaining coexistence review findings without touching the original v11 database.
2. Finish and independently review Task 6 aggregation and nine Tauri commands.
3. Keep explicit read-only snapshot import deferred unless it becomes necessary for the accepted product flow.
4. Implement Task 7 frontend.
5. Implement Task 8 mock-upstream E2E, hidden legacy UI, acceptance documentation, and full backend/frontend gate.
