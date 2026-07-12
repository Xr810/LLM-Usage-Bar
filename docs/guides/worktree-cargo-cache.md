# Worktree-safe Cargo build cache

Local Rust and Tauri commands use a shared Cargo target cache without allowing
one worktree's dependency graph to overwrite another's build artifacts. The
wrapper sets `CARGO_TARGET_DIR`; do not set it manually for normal local work.

## Local commands on macOS, Windows, and Linux

The same `pnpm` commands work in macOS or Linux shells, Windows PowerShell, and
Windows Command Prompt:

| Task | Command |
| --- | --- |
| Run Cargo | `pnpm rust -- <cargo arguments>` |
| Run Tauri | `pnpm tauri -- <arguments>` |
| Start Tauri development | `pnpm dev` |
| Build the Tauri application | `pnpm build` |
| Inspect cache status | `pnpm cargo:cache -- status` |

For example, use `pnpm rust -- check`, `pnpm rust -- test`, or
`pnpm rust -- clippy`. Do not run local `cargo build`, `cargo test`,
`cargo clippy`, or `tauri` directly because those commands bypass the target
selection and lease protections.

## Lock-hash isolation

Each invocation hashes the current worktree's `src-tauri/Cargo.lock` and uses:

```text
<git-common-root>/.cache/cargo-targets/<Cargo.lock SHA-256>/
```

Worktrees with identical `Cargo.lock` contents share the same bucket. Worktrees
with different lockfile contents use different buckets. Changing a lockfile
selects a new bucket; it does not delete the old one. The cache directory is
local-only and ignored by Git.

## Leases fail closed

The wrapper creates a lease before starting Cargo or Tauri and records the
spawned build process tree. Cache inspection keeps every bucket that is still
referenced by a registered worktree or has an active, pending, current-boot
orphan, malformed, or otherwise ambiguous lease. A valid previous-boot orphan
may be removed only when its boot identity proves that the old process tree has
ended and no worktree references the bucket. If Git cannot enumerate every
worktree, a lockfile cannot be read safely, or process-tree state cannot be
proved inactive, pruning stops or keeps the bucket. Uncertainty never permits
deletion.

## Inspecting and pruning

`pnpm cargo:cache -- status` is read-only. It scans registered worktrees and
reports the current worktree lock hash, every referenced lock hash, and the
keep/remove plan even when the current cache bucket has not been materialized.

`pnpm cargo:cache -- prune` is also read-only and prints a dry-run plan. Review
that plan before running `pnpm cargo:cache -- prune --apply`. Apply mode deletes
only unreferenced hash buckets whose leases are provably inactive. It checks all
worktrees and lockfiles before the first deletion, rechecks immediately before
each deletion, and stops on the first unsafe state or error.

Before removing a worktree, run `pnpm cargo:cache -- status` while that
worktree and its `Cargo.lock` are still present. If status fails or reports an
unsafe state, stop and investigate. After stopping its builds and removing the
worktree, run the dry-run prune command from a remaining worktree before using
`--apply`.

## CI exemption

This wrapper policy applies to local development, where multiple persistent
worktrees share one repository cache. CI jobs are exempt: existing CI scripts
may invoke Cargo or Tauri directly when the runner already provides isolated
build directories and owns its cache lifecycle. Do not point concurrent CI
jobs at this local shared cache without the wrapper and lease protocol.
