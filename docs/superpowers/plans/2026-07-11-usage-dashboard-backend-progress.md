# Usage Dashboard Backend Implementation Progress

Last updated: 2026-07-11

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

- [x] Coexistence safety: isolate the Usage Dashboard from original CC Switch (`e931e7f9`).
  - The original installation supports database schema v11 and currently uses `~/.cc-switch/cc-switch.db`.
  - This branch must not start against or upgrade that database to v13.
  - Implemented distinct product/bundle identity, `~/.llm-usage-bar`, no legacy deep-link registration/updater, unsafe-override rejection, and proxy port 15722.
  - Tests proved the original v11 database hash, mtime, size, and version stay unchanged.
  - Optional original-data import is not implemented; if added, it requires explicit user action and a read-only SQLite Backup snapshot.

- [ ] Task 3: static one-Provider request routing.
  - Main implementation commit: `f323ea75`
  - Full Rust suite passed before review.
  - Independent review found three follow-ups: validate legacy route config before forwarding, preserve both global Usage Provider ID and legacy runtime Provider ID, and prove local 503 produces zero upstream hits in the final mock-upstream acceptance test.
  - Review fixes were implemented in `7fafece4` and `73ccb294` and are being independently reverified. The request-level zero-hit proof is part of Task 8.
- [ ] Task 4: transactional usage ingestion and trusted upstream-cost capture.
  - Implementation commit: `7fafece4`
  - Targeted verification and independent review are in progress.
  - New events must use the global Usage Provider ID; compatibility logs may retain the legacy Provider ID.

- [x] Task 5: Provider-aware quota collection and bound Session import (`fc57e960`).
  - Targeted quota, Session service, and Claude/Codex parser tests pass.
  - Independent review is in progress.

## Not started

- [ ] Task 6: product aggregation and nine Tauri commands.
- [ ] Task 7: minimal React dashboard and Provider/route configuration UI.
- [ ] Task 8: hide legacy entry points, real mock-upstream proxy acceptance, full backend/frontend gate, and acceptance runbook.

## Completion rule

The plan is not complete until Tasks 1 through 8 are implemented, reviewed, and the final Rust, TypeScript, unit-test, renderer-build, static-route, and mock-upstream acceptance gates all pass.
