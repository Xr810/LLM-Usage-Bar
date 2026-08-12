# Task 8 Routing Isolation and Regression Acceptance

## Outcome

Task 8 completes the Agent-centric usage plan with local-only protocol acceptance,
setup-path normalization, public-view consistency fixes, unavailable-credential
cleanup, full regression verification, and independent functional review.

## Accepted behavior

- Claude, Codex, and Gemini requests reach a local mock upstream through the real
  proxy lifecycle and preserve the exact Agent owner selected by each binding.
- Two bindings on one Provider remain independently attributable. Later rebind,
  rotation, deletion, or Agent archival cannot rewrite earlier event ownership.
- Unknown, disabled, cleared, archived, store-missing, and mismatched bindings stop
  locally with no upstream request.
- Setup URLs work as published for Claude, Codex, and Gemini. Only direct API-key
  bindings publish a local base URL and credential placement; managed and unsupported
  bindings retain their protocol/status metadata without unusable setup instructions.
- Public events, diagnostics, SQL export, application logs, captured URI/body data,
  and frontend DOM/form/QueryCache/MutationCache snapshots omit all fixture binding
  values. Raw key material remains outside React Query and SQLite.
- Provider list, save, and dashboard responses expose the same verified binding
  status. Session-only bindings remain non-effective for shared-account derivation.
- An unavailable binding exposes Clear only when both protected-store metadata
  pointers still exist. Clear uses the frozen credential version, and conflict
  failures refresh root query state without replacing the original error.

## Review findings resolved

- The Settings snapshot now includes Portal content, current form values, QueryCache,
  and MutationCache rather than only the render container.
- Application log capture is exercised by the local proxy tests.
- Gemini ownership is asserted per request using distinct token results, so swapped
  Agent ownership cannot pass as an unordered set.
- Claude rebind acceptance now runs the same public snapshot check as Codex and
  Gemini.
- Public Provider DTOs are hydrated from the verified binding service at the command
  boundary, including nested dashboard Provider views.
- `canClearCredential` separates removable unavailable metadata from an empty
  unavailable binding, with positive and negative UI tests.
- Persisted Agent selection survives the initial empty query state. Metered shared
  accounts, fixture effective state, Provider counts, and compatibility-manifest
  locations now match production semantics.
- Claude and Gemini remove their local namespace before forwarding, Codex serves its
  namespaced model catalog, and Claude Desktop publishes only its binding-key header.

## Verification

- `pnpm test:unit` — PASS, 92 files / 513 tests.
- `pnpm typecheck` — PASS.
- `pnpm build:renderer` — PASS.
- `pnpm format:check` — PASS.
- `pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check` — PASS.
- `pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` — PASS.
- `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib` — PASS, 2201 passed / 2 ignored.
- `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --tests` — PASS, library
  result repeated plus 141/141 integration tests; local proxy target 20/20.
- `git diff --check` — PASS.
- Task 8 frontend and backend functional reviews — APPROVED, no remaining P0-P2.

## Known baseline advisories

- Node prints the existing `punycode` deprecation advisory during Vitest.
- Vite reports the existing post-minification chunk-size advisory above 500 kB.
- The local Cargo target wrapper prints a sandbox `sysctl` permission advisory;
  format, Clippy, and test commands still exit zero.

## Isolation

- Tests use in-memory or temporary databases and operating-system-assigned localhost
  ports. They do not start the desktop app or use the real application data path.
- Work remains on `codex/agent-centric-usage-modules` in the linked worktree.
- The root checkout's user-owned Settings edits and `.pnpm-store/` are untouched.
