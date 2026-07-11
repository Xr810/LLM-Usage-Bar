# Usage Dashboard Backend Implementation Progress

Last updated: 2026-07-12 00:42 Asia/Singapore

Source plan: `2026-07-11-usage-dashboard-backend-implementation.md`

Execution plan: `2026-07-11-usage-dashboard-backend-execution.md`

## Completed

- [x] Task 1: v13 domain model and v12 to v13 migration.
  - Commits: `55814021`, `1fcaf35d`
  - Independent review: approved.
- [x] Task 2: Provider, route/source binding, immutable event/link, and quota persistence DAO.
  - Commits: `ac70ec35`, `18e146fe`
  - Independent review: approved.

## In progress

- [x] Coexistence safety: isolate the Usage Dashboard from original CC Switch (`e931e7f9`, `7fb6a6df`).
  - The original installation supports database schema v11 and currently uses `~/.cc-switch/cc-switch.db`.
  - This branch must not start against or upgrade that database to v13.
  - Implemented distinct product/bundle identity, `~/.llm-usage-bar`, no legacy deep-link registration/updater, unsafe-override rejection, and proxy port 15722.
  - Tests use fake/in-memory homes and do not start this app against the original v11 database. The running original app legitimately updates its own database, so long-window hash equality is not used as the final proof.
  - Optional original-data import is not implemented; if added, it requires explicit user action and a read-only SQLite Backup snapshot.
  - Independent review gaps were fixed: Windows/tray labels and frontend fallback paths are isolated, and symlink plus macOS case-alias regression tests pass. Independent re-review approved the boundary.

- [x] Task 3: static one-Provider request routing.
  - Main implementation commit: `f323ea75`
  - Full Rust suite passed before review.
  - Independent review found three follow-ups: validate legacy route config before forwarding, preserve both global Usage Provider ID and legacy runtime Provider ID, and prove local 503 produces zero upstream hits in the final mock-upstream acceptance test.
  - Review fixes were implemented in `7fafece4` and `73ccb294` and independently approved.
  - Task 8 acceptance (`4fa50dea`) proves that a route-less request stays local 503 with zero upstream hits even when reachable legacy current/failover candidates exist. `0e27e91c` removes the unused legacy selector from the three request-path files; the required static search has no matches.
- [x] Task 4: transactional usage ingestion and trusted upstream-cost capture.
  - Implementation commit: `7fafece4`
  - Review fixes: `73ccb294`; targeted verification and independent re-review approved.
  - New events must use the global Usage Provider ID; compatibility logs may retain the legacy Provider ID.

- [x] Task 5: Provider-aware quota collection and bound Session import (`fc57e960`, `49907148`, `958a1cf6`).
  - Review fixes persist every failed attempt, enforce one Provider per machine-local quota source, and use explicit source bindings as the only Session sync source of truth.
  - Targeted quota (8), Session service (4), and underlying Session parser/service (36) tests pass.
  - Binding-change TOCTOU review finding fixed; normalization and ownership tests pass. Independent re-review approved.

- [x] Task 6: product aggregation and nine Tauri commands (`fd8f7af6`, `2194c36d`).
  - Product/Provider aggregation, duplicate exclusion, Decimal cost sums, half-open filters, pagination, redacted quota/fetch state, and exactly nine registered commands are implemented.
  - Review fixes add checked Decimal sums, immutable historical product grouping, consistent/monotonic quota state, and a redacted refresh result.
  - Targeted tests pass (quota DAO 5, quota service 9, dashboard 6, command adapters 2); the complete Rust suite passes (1850 passed, 2 ignored plus all integration binaries).
  - Independent re-review approved.

- [ ] Task 7: minimal React dashboard and Provider/route configuration UI.
  - Initial TDD implementation: `f9063ff8`; Important review fixes: `66cff322`.
  - Target tests pass 15/15, TypeScript typecheck and renderer build pass.
  - Independent re-review confirmed every Important finding and exact date-range coverage are fixed. Two Minor findings (localized unavailable event source and visible/disabled proxy-state failure handling) are being closed in the Task 8 frontend commit.

- [ ] Task 8: hide legacy entry points, real mock-upstream proxy acceptance, full backend/frontend gate, and acceptance runbook.
  - Main-path dashboard commit: `ee93e413`; compatibility source modules remain compiled but legacy entry points are no longer rendered.
  - Real proxy acceptance: `15e44174`, hardened and independently approved in `4fa50dea`.
  - Acceptance runbook: `9575ab78`.
  - Request-path static selector removal: `0e27e91c`; Provider router tests 11/11 and required `rg` zero-match gate pass.
  - Frontend review found the minimal desktop window shell, v13 event invalidation and non-self-proving App/full-suite tests still need repair. Those fixes are in progress; the full final gate has not run yet.

## Completion rule

The plan is not complete until Tasks 1 through 8 are implemented, reviewed, and the final Rust, TypeScript, unit-test, renderer-build, static-route, and mock-upstream acceptance gates all pass.
