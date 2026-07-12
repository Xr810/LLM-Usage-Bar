# LLM Usage Bar Full Identity, Session Watcher, and Build Cache Migration Design

**Status:** Approved direction; written specification awaiting final user review
**Base commit:** `66cd86a5` (`codex/integration-pr4-pr5-pr6`)
**Implementation branch:** `codex/full-identity-sync-cache-migration`

## Purpose

Complete the separation of LLM Usage Bar from CC Switch, replace line-based
session import cursors with durable byte/parser cursors, introduce a
rate-limited filesystem watcher without feedback loops, and prevent Cargo
artifacts from multiplying across Git worktrees.

The work is intentionally split into three independently reviewable
subprojects:

1. Product identity and database migration.
2. Durable session cursors and the five-minute watcher runtime.
3. Worktree-aware Cargo cache routing and pruning.

Each subproject must be testable and committable on its own. The final desktop
smoke test covers their combined behavior.

## Current Evidence

- The Tauri product and bundle identifier are already `LLM Usage Bar` and
  `com.llmusagebar.desktop`, but the npm package, Cargo package, Rust library,
  development executable, log file, updater links, packaging metadata, and
  many visible strings still use CC Switch identity.
- The current database lives under `~/.llm-usage-bar` but is still named
  `cc-switch.db`.
- Claude and Codex importers persist line offsets. Claude skips old lines but
  still reads them; Codex must replay old lines to reconstruct cumulative token
  state. The periodic task performs synchronous filesystem and SQLite work on
  an async task.
- Four independent worktree `target` directories previously accounted for
  almost all repository disk usage. They have now been safely cleaned.
- The project has no committed Cargo target routing policy, so a future direct
  build in each worktree would recreate the same problem.

## Global Safety Constraints

1. Never write to, rename, migrate in place, or take ownership of the original
   `~/.cc-switch` directory or its `cc-switch.db` database.
2. The only automatically migrated legacy database is
   `~/.llm-usage-bar/cc-switch.db`, which belongs to the new application but
   retained the old filename during development.
3. Never copy a live SQLite database with `fs::copy`. Use SQLite's Backup API,
   validate the destination, and publish it with an atomic rename.
4. No watcher callback may read session files, query SQLite, write application
   state, or start a sync. Callbacks may only classify an event and mark a
   source dirty through a bounded channel.
5. A source may begin at most one automatic sync in any five-minute window.
6. Automatic synchronization must never overlap for the same source and must
   not catch up missed timer ticks in a burst.
7. A cursor advances only after every yielded usage record and the associated
   database transaction have succeeded.
8. `main` and its existing uncommitted `package.json` change remain untouched.
9. Historical plan and task-state documents remain historical records; they
   are not rewritten to pretend old executable names never existed.

## 1. Identity Namespace Model

### 1.1 Owned identity

Introduce one source of truth for current product identity:

| Field | Value |
|---|---|
| Display name | `LLM Usage Bar` |
| Slug / npm package / Cargo package | `llm-usage-bar` |
| Rust library crate | `llm_usage_bar_lib` |
| Bundle identifier | `com.llmusagebar.desktop` |
| Database filename | `llm-usage-bar.db` |
| Application log basename | `llm-usage-bar` |
| Repository | `Xr810/LLM-Usage-Bar` |
| Default local proxy port | `15722` |

All current-product UI copy, diagnostics, auto-start entries, packaging
metadata, executable references, backup headers, and test fixtures use the
owned identity.

### 1.2 Explicit legacy namespace

The string `CC Switch` or slug `cc-switch` remains only where it identifies an
external legacy format or the original application:

- Read-only import source `~/.cc-switch`.
- Old SQL export headers accepted for import.
- Old provider/config formats whose serialized identifier is defined by CC
  Switch rather than this application.
- Upstream issue references explaining inherited protocol behavior.
- Migration labels such as `Legacy CC Switch` shown to users.

Every remaining runtime occurrence must be paired with a `legacy`-named
constant, enum variant, function, or explanatory comment. Unqualified current
product identifiers named `cc_switch` are not allowed after the migration.

### 1.3 Updater and release boundary

Inherited updater fallbacks must stop opening or checking the public CC Switch
release feed. Until LLM Usage Bar has a reachable release endpoint, update
checks report that managed updates are unavailable and do not redirect to the
upstream project. Repository, support, security, Flatpak, and installer
metadata point to `Xr810/LLM-Usage-Bar`.

### 1.4 App-owned internal identifiers

Schema v14 performs an explicit, column-by-column migration of app-owned
discriminators, settings keys, storage-location enum values, auto-start labels,
and current-product profile names from `cc-switch` to `llm-usage-bar`. Code
variants such as `SkillStorageLocation::CcSwitch` become LLM Usage Bar variants.

This is not a global text or SQL replacement:

- Provider/source IDs such as `claude`, `codex`, `gemini`, `opencode`, and
  `kimi` retain their protocol meaning.
- Rows that identify imported original-product data become the explicit value
  `legacy-cc-switch` instead of being relabeled as current-product data.
- Serialized third-party compatibility values remain unchanged when the
  external consumer requires the old value.
- The migration enumerates every affected table and column in code; JSON blobs
  are decoded and rewritten through typed structures rather than string
  substitution.

Tests assert both sides of this boundary: no app-owned current value remains
`cc-switch`, and no approved legacy value is accidentally rewritten.

## 2. Database File Identity Migration

Run the filename migration before normal database initialization.

### 2.1 Decision table

| New file | Old current-app file | Behavior |
|---|---|---|
| Exists | Any state | Open `llm-usage-bar.db`; never overwrite it. Report the old file as a retained migration artifact if present. |
| Missing | Missing | Create a fresh `llm-usage-bar.db`. |
| Missing | Exists | Back up the old current-app database into a temporary new database, validate it, atomically publish the new filename, then archive the old filename. |

The old current-app path means only
`~/.llm-usage-bar/cc-switch.db`. Path canonicalization and Unix object identity
checks reject aliases that resolve into `~/.cc-switch`.

### 2.2 Backup protocol

1. Open the source read-only with SQLite.
2. Use `rusqlite::backup` to create a temporary destination in the same
   directory. This captures committed WAL content through SQLite rather than
   copying sidecar files manually.
3. Run `PRAGMA quick_check`, read `user_version`, and verify required tables.
4. Flush and atomically rename the temporary destination to
   `llm-usage-bar.db`.
5. Rename the old current-app file to
   `cc-switch.db.pre-llm-usage-bar-v14`. Do not delete it automatically.
6. If any step fails, remove only the incomplete temporary destination, leave
   the source untouched, and surface a recoverable initialization error.

If both filenames exist, the new filename is authoritative. The application
never attempts a merge between two databases.

## 3. Schema v14 Durable Sync Cursors

Increase `SCHEMA_VERSION` from 13 to 14 and replace `session_log_sync` with a
source-aware cursor table.

### 3.1 Table

`usage_sync_cursors` contains:

- `source TEXT NOT NULL`
- `cursor_key TEXT NOT NULL`
- `resource_path TEXT`
- `resource_identity TEXT`
- `modified_at_ns INTEGER NOT NULL DEFAULT 0`
- `size_bytes INTEGER NOT NULL DEFAULT 0`
- `byte_offset INTEGER NOT NULL DEFAULT 0`
- `line_offset INTEGER NOT NULL DEFAULT 0`
- `parser_state_json TEXT`
- `last_success_at INTEGER NOT NULL DEFAULT 0`
- primary key `(source, cursor_key)`

`resource_identity` records stable filesystem identity where the platform
provides it. A size decrease or identity change means rotation/truncation and
forces a safe replay.

`parser_state_json` is a versioned payload. Codex stores the session ID,
current model, previous cumulative totals, and event index required to resume
delta calculation. Claude stores the current session ID needed by appended
records. Gemini and OpenCode may leave parser state empty and use their current
mtime/session-watermark algorithms.

### 3.2 v13 migration

Rename the old table to `session_log_sync_v13_archive` inside the v13-to-v14
transaction. Classify known paths against Claude, Codex, Gemini, and OpenCode
roots and insert conservative v14 rows:

- Preserve the old line offset.
- Set byte offset and parser state to zero/empty because they cannot be derived
  safely.
- Mark unclassified rows with source `legacy` so they cannot suppress a known
  source scan.

The first successful v14 scan reconstructs parser state, calculates the byte
offset, and replaces the conservative cursor. Existing event IDs and database
deduplication prevent duplicate usage rows during that one-time replay.

## 4. Incremental Parser Contract

### 4.1 Append fast path

For Claude and Codex, seek directly to `byte_offset` only when all of these are
true:

- Resource identity matches.
- File size is at least the stored size and byte offset.
- The database cursor line offset matches the parser state's line offset.
- Parser state version is supported.

Parse appended complete lines only. An incomplete final line remains
uncommitted and is retried later.

### 4.2 Replay path

On cache miss, migration, truncation, replacement, unsupported state, or
cursor inconsistency, replay from byte zero. Replay may restore parser state
from all prior lines but emits records only after the stored legacy line
offset. Deduplication remains the final correctness guard.

### 4.3 Commit order

1. Parse into an in-memory batch.
2. Ingest the complete batch transactionally.
3. Persist the new v14 cursor.
4. Publish the sync result.

Failures in steps 1-3 leave the old cursor unchanged.

## 5. Filesystem Watcher Without Feedback Loops

Use `notify` 8.2 through `recommended_watcher`, which selects the native
backend on macOS, Windows, and Linux. The watcher is a hint source, not the
source of truth; native watchers can miss events on some filesystems, so a
periodic reconciliation remains required.

### 5.1 Watched roots

Watch only registered external usage sources:

- Claude project transcript roots.
- Codex active and archived session roots.
- Gemini chat roots.
- The OpenCode database and WAL parent directory with exact filename filters.

Never watch:

- `~/.llm-usage-bar` or `~/.cc-switch`.
- The application database, logs, backups, or crash reports.
- The Git repository, `.worktrees`, `target`, `node_modules`, or shared Cargo
  cache.
- A symlink/case alias that resolves into an excluded root.

### 5.2 Callback behavior

The native callback:

1. Rejects access-only events and paths outside the source allowlist.
2. Maps accepted paths to a source ID.
3. Attempts to enqueue the source ID into a bounded channel.
4. On channel overflow, sets one atomic `dirty_all` flag.
5. Returns immediately.

It never calls a sync function and never writes a file. Reads performed by a
sync therefore cannot recursively schedule another sync.

### 5.3 Five-minute state machine

Each source has `Clean`, `Dirty`, or `Syncing` state plus `next_allowed_at`.

- Startup performs one background reconciliation after application state and
  the UI are ready; that run starts the first five-minute window.
- Watcher events coalesce into `Dirty`.
- A scheduler tick every five minutes uses
  `tokio::time::MissedTickBehavior::Skip`.
- At a tick, a source starts only if dirty and not already syncing.
- An event received during `Syncing` leaves the source dirty, but the follow-up
  cannot start until the next five-minute tick.
- Success clears dirty state only if no newer event arrived.
- Failure keeps the source dirty and retries no sooner than the next tick.
  Repeated failures back off to 10, 20, then 30 minutes.
- Watcher error or queue overflow marks all sources dirty for the next tick; it
  never launches an immediate full scan.

The scheduler performs a lightweight metadata reconciliation every five
minutes. Parser work still occurs only for resources whose cursor metadata
changed. This reconciliation closes watcher-loss gaps without creating a hot
polling loop.

All synchronous file parsing and external SQLite queries run in
`tauri::async_runtime::spawn_blocking`. Manual provider sync uses the same
per-source gate, returns a visible `sync already in progress` result instead
of blocking, and resets the five-minute window on success.

## 6. Watcher Lifecycle

Add a `SessionWatcherService` and `SessionWatcherHandle` beside the existing
quota scheduler pattern.

- `AppState` owns the session service and one optional watcher handle.
- Startup constructs watch roots only after database/config initialization.
- Provider source-binding changes update watched roots through a serialized
  command; no second watcher is created.
- Shutdown cancels the scheduler, closes the event channel, unwatches roots,
  waits for in-flight blocking work, and then drops the native watcher.
- Poisoned locks and watcher backend errors are logged and reflected in a
  diagnostic status DTO; they never restart recursively.

## 7. Worktree-Aware Cargo Cache

### 7.1 Routing

Add a cross-platform Node wrapper that:

1. Resolves the absolute Git common directory.
2. Hashes `src-tauri/Cargo.lock` with SHA-256.
3. Sets `CARGO_TARGET_DIR` to
   `<main-repo>/.cache/cargo-targets/<lock-hash>`.
4. Writes an active-process lease containing PID, start time, worktree, and
   command.
5. Spawns Cargo or Tauri with inherited stdio and removes the lease on exit.

Worktrees with the same dependency lock reuse one cache. Dependency-upgrade
branches receive a separate bucket and therefore cannot poison or serialize
unrelated dependency graphs.

`package.json` routes `dev`, `build`, and `tauri` through the wrapper and adds a
`rust` script for local Cargo commands. CI may continue using direct Cargo in
its ephemeral checkout.

### 7.2 Pruning

Add a cache-management command with these invariants:

- Default mode is status/dry-run.
- `--apply` deletes only cache hashes not referenced by any current worktree.
- A bucket with a live lease is never deleted.
- Stale leases are removed only after their PID/start identity is invalid.
- Legacy per-worktree `src-tauri/target` directories are reported but never
  automatically deleted by the prevention script.
- The command never touches `.cargo`, `.rustup`, application data, or source
  worktrees.

Commit `.cache/cargo-targets/` to `.gitignore`. Add a repository `AGENTS.md`
rule requiring local agents to use the wrapper for Cargo/Tauri builds and to
run the dry-run report before removing a worktree. Preserve the managed Kimi
delegation block verbatim in that file.

## 8. Implementation Boundaries

### Subproject A: Identity and database filename

- Central identity constants and compatibility constants.
- npm/Cargo/library/binary/log rename.
- Current-app database Backup API migration.
- App-owned UI, localization, packaging, docs, updater, auto-start, and test
  fixture migration.
- Explicit `Legacy CC Switch` labels at compatibility boundaries.

### Subproject B: Schema v14 and watcher

- v14 cursor migration and DAO.
- Claude/Codex byte/parser cursors.
- Five-minute watcher state machine and lifecycle.
- Blocking-pool and manual-sync integration.
- Gemini/OpenCode keep their established parsing semantics while adopting the
  shared scheduler and v14 cursor API.

### Subproject C: Cargo cache policy

- Cross-platform target router and active leases.
- Cache status/prune command.
- Package scripts, ignore rule, agent rule, and developer documentation.

No subproject may include unrelated provider, proxy-routing, pricing, or UI
redesign work.

## 9. Error Handling and Rollback

- Database filename or v14 migration failure leaves the old database and v13
  archive intact and shows a recoverable startup error.
- Watcher initialization failure degrades to five-minute reconciliation only.
- Watcher runtime failure marks all sources dirty for the next scheduled
  reconciliation and exposes diagnostics.
- Cursor decode failure replays that resource from zero; it does not abort
  other sources.
- Cargo wrapper failure to resolve Git state exits before running the build; it
  never silently falls back to a per-worktree target.
- A failed cache prune stops at the first error and reports every path already
  removed. It never retries deletion in a loop.

Rollback is commit-scoped: identity, watcher/schema, and cache policy land as
separate commits. The archived old database filename and v13 cursor table
provide forensic recovery, but an older application must not open a v14
database because the existing future-version guard will reject it.

## 10. Test Strategy

### Identity and database tests

- Public identity manifests agree on `llm-usage-bar` / `LLM Usage Bar`.
- Runtime executable and library crate names are distinct and correct.
- Current-app old filename migrates through SQLite Backup API, including WAL
  content.
- New filename wins when both exist.
- Migration failure leaves the old file byte-for-byte intact.
- Any alias resolving to `~/.cc-switch` is rejected.
- Visible localizations contain no unqualified current-product `CC Switch`
  copy; approved legacy contexts remain.

### Cursor tests

- v13 rows migrate conservatively to v14.
- Claude append reads from the stored byte offset.
- Codex append restores cumulative token state and emits the exact delta.
- Truncation, replacement, unsupported state, and cursor mismatch replay
  safely.
- Partial final lines and ingestion failures do not advance the cursor.
- Restart from persisted parser state produces the same totals as a full
  replay.

### Watcher tests

- Event storms coalesce to one dirty source.
- Application data, repository, cache, and legacy data paths are rejected.
- A sync-generated application database/log write cannot map to a watched
  source.
- No source starts twice inside five minutes.
- Events during a sync wait for the next tick.
- Missed ticks skip rather than burst.
- Repeated failures back off and never enter a tight retry loop.
- Overflow and backend errors schedule one future reconciliation.
- Watcher shutdown is idempotent.

OS watcher tests use a temporary external source directory for one smoke-level
test. Timing/state-machine unit tests use a fake event source and controllable
clock so the suite is deterministic.

### Cargo cache tests

- Main checkout and linked worktrees with the same lockfile resolve the same
  target directory.
- A different lockfile resolves a different target directory.
- Dry-run never removes files.
- Apply mode retains active and referenced buckets and removes only inactive,
  unreferenced buckets.
- Wrapper exit codes and termination signals propagate to the child command.

### Final gates

- Rust format and Clippy with warnings denied.
- Full Rust unit, integration, and proxy E2E suite.
- Frontend typecheck, format check, unit suite, and renderer build.
- Cache-script Node tests on macOS and CI-supported Windows/Linux runners.
- Isolated-HOME desktop smoke: v13 current-app filename migration to v14,
  watcher diagnostics, five-minute state, Dashboard rendering, manual sync,
  clean shutdown, and no access to real `~/.cc-switch`.

## 11. Acceptance Criteria

1. Activity Monitor and built artifacts identify the development application
   as LLM Usage Bar, not CC Switch.
2. All app-owned database, log, package, library, packaging, and current UI
   identity is migrated; legacy naming exists only behind explicit legacy
   boundaries.
3. A v13 current-app database migrates to `llm-usage-bar.db` and schema v14
   without losing committed WAL data or touching original CC Switch data.
4. Claude and Codex append sync avoids replaying unchanged bytes during normal
   operation.
5. Watcher event storms, self-output, failures, and missed ticks cannot cause a
   recursive or tight synchronization loop.
6. Automatic sync frequency is no more than once per source per five minutes.
7. Same-lock worktrees reuse one Cargo target bucket; different locks remain
   isolated.
8. Cache cleanup is dry-run by default and refuses active/referenced buckets.
9. Existing `main/package.json` changes and all original CC Switch user data
   remain untouched.

## 12. External Reference

The watcher design uses the cross-platform `notify` 8.2
`recommended_watcher` API and deliberately retains reconciliation because the
official documentation notes that native filesystem notifications may miss
events on some filesystems or very large watched sets:

- <https://docs.rs/notify/latest/notify/>
- <https://docs.rs/notify/latest/notify/type.RecommendedWatcher.html>
