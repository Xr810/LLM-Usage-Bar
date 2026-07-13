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
| 1. v16 schema and conservative history migration | pending | — | — |
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
