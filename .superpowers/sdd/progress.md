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
| 2. Agent module and binding persistence | pending | — | — |
| 3. Protected credential store and atomic key lifecycle | pending | — | — |
| 4. Credential-routed proxy and frozen event attribution | pending | — | — |
| 5. Trusted session attribution and Agent-safe dedup | pending | — | — |
| 6. Agent dashboard, events, diagnostics, commands | pending | — | — |
| 7. Agent-centric frontend and Settings | pending | — | — |
| 8. Integration, security review, and full acceptance | pending | — | — |

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
