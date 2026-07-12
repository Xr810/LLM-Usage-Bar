# Schema v14 Incremental Session Watcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace line-only session sync state with durable source-aware byte/parser cursors and run external-source synchronization through one macOS filesystem watcher whose per-source interval is configurable and defaults to five minutes.

**Architecture:** Schema v14 introduces `usage_sync_cursors` and archives v13 state conservatively. Claude/Codex resume append parsing from byte offsets with versioned parser state; Gemini/OpenCode retain their format-specific semantics behind one cursor DAO. A pure rate-limit state machine accepts bounded watcher hints, while native `notify` callbacks only mark sources dirty; blocking parsers run behind per-source gates and one service-wide concurrency permit.

**Tech Stack:** Rust 1.95, rusqlite, serde_json, Tauri async runtime, Tokio, notify 8.2.0, tempfile-based tests.

## Global Constraints

- Use `pnpm rust -- ...` for every local Rust command.
- The supported runtime and watcher target is macOS only.
- Increase schema from 13 to 14 exactly once; v14 must keep `session_log_sync_v13_archive` for forensic recovery.
- Cursor primary key is `(source, cursor_key)` and includes resource identity, mtime ns, size, byte/line offset, versioned parser state, and last success time.
- Cursor advances only after every parsed record is ingested successfully.
- Real `~/.cc-switch`, application data, logs, repository, worktrees, node modules, and Cargo caches are never watched.
- Nested external source roots are watched recursively; excluded output roots are omitted entirely rather than recursively filtered after registration.
- Native callbacks only perform lexical classification against prevalidated watched roots and enqueue dirty source IDs through a bounded channel; filesystem identity checks happen in workers before parsing.
- Automatic sync uses one persisted global interval: default 5 minutes, valid range 1-1440 minutes; each source begins at most one automatic sync per configured window.
- Tokio missed ticks use `MissedTickBehavior::Skip`; no burst catch-up.
- Events during sync wait until a future configured tick.
- With the default interval, failures back off 5, 10, 20, then 30 minutes; custom intervals back off by 1x/2x/4x/6x and never tight-loop.
- All synchronous file/SQLite source parsing runs in `spawn_blocking`.
- Exactly one watcher scheduler replaces the old 60-second loop; automatic parser concurrency is one job, and startup reconciliation begins only after an explicit UI-ready signal.
- Tests use real parser/filesystem behavior and a fake clock/event source; do not add production APIs used only by tests.

---

### Task 1: Add schema v14 cursor domain and DAO

**Files:**
- Create: `src-tauri/src/database/dao/usage_sync_cursors.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/tests.rs`

**Interfaces:**
- Produces `UsageSyncCursor`, `get_usage_sync_cursor(source, key)`, `list_usage_sync_cursors(source)`, `put_usage_sync_cursor(cursor)`, `put_usage_sync_cursor_on_conn(conn, cursor)`, and `delete_usage_sync_cursor(source, key)`; the connection-level form enables one transaction with usage ingestion, while the source-scoped list drives periodic metadata reconciliation without reading the archived v13 table.

- [ ] **Step 1: Write failing DAO and schema tests**

```rust
#[test]
fn v14_cursor_round_trips_every_resume_field() {
    let db = Database::memory().unwrap();
    let cursor = UsageSyncCursor {
        source: "codex".into(),
        cursor_key: "session.jsonl".into(),
        resource_path: Some("/tmp/session.jsonl".into()),
        resource_identity: Some("unix:1:2".into()),
        modified_at_ns: 11,
        size_bytes: 22,
        byte_offset: 17,
        line_offset: 3,
        parser_state_json: Some(r#"{"version":1,"eventIndex":2}"#.into()),
        last_success_at: 33,
    };
    db.put_usage_sync_cursor(&cursor).unwrap();
    assert_eq!(db.get_usage_sync_cursor("codex", "session.jsonl").unwrap(), Some(cursor));
}

#[test]
fn cursor_update_is_atomic_and_source_scoped() {
    let db = Database::memory().unwrap();
    let mut claude = cursor_fixture("claude", "same.jsonl", 10);
    let codex = cursor_fixture("codex", "same.jsonl", 20);
    db.put_usage_sync_cursor(&claude).unwrap();
    db.put_usage_sync_cursor(&codex).unwrap();
    claude.byte_offset = 30;
    db.put_usage_sync_cursor(&claude).unwrap();
    assert_eq!(db.get_usage_sync_cursor("claude", "same.jsonl").unwrap(), Some(claude));
    assert_eq!(db.get_usage_sync_cursor("codex", "same.jsonl").unwrap(), Some(codex));
}
```

Define `cursor_fixture(source, key, byte_offset)` in the test module and populate every field, so the test exercises the real `INSERT ... ON CONFLICT` statement without mocks. Add a source-scoped list test proving reconciliation sees every cursor for one source and no cursor owned by another source.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage_sync_cursor -- --nocapture`

Expected: FAIL because the table/domain/DAO do not exist and schema version is 13.

- [ ] **Step 3: Implement table and DAO**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSyncCursor {
    pub source: String,
    pub cursor_key: String,
    pub resource_path: Option<String>,
    pub resource_identity: Option<String>,
    pub modified_at_ns: i64,
    pub size_bytes: i64,
    pub byte_offset: i64,
    pub line_offset: i64,
    pub parser_state_json: Option<String>,
    pub last_success_at: i64,
}
```

```sql
CREATE TABLE IF NOT EXISTS usage_sync_cursors (
  source TEXT NOT NULL,
  cursor_key TEXT NOT NULL,
  resource_path TEXT,
  resource_identity TEXT,
  modified_at_ns INTEGER NOT NULL DEFAULT 0,
  size_bytes INTEGER NOT NULL DEFAULT 0,
  byte_offset INTEGER NOT NULL DEFAULT 0,
  line_offset INTEGER NOT NULL DEFAULT 0,
  parser_state_json TEXT,
  last_success_at INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (source, cursor_key)
);
```

Use a single `INSERT ... ON CONFLICT(source,cursor_key) DO UPDATE` statement assigning every field. Reject negative sizes/offsets before SQL. Set `SCHEMA_VERSION = 14` only after Task 2 adds the migration path; until then focused tests may initialize the table through `create_tables` while migration test remains RED.

- [ ] **Step 4: Verify focused GREEN**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage_sync_cursor -- --nocapture`

Expected: DAO tests pass; the explicit v13-to-v14 migration test is still pending Task 2.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/database/dao/usage_sync_cursors.rs src-tauri/src/database/dao/mod.rs src-tauri/src/database/schema.rs src-tauri/src/database/mod.rs src-tauri/src/database/tests.rs
git commit -m "feat(usage): add durable sync cursor storage"
```

### Task 2: Migrate v13 line state conservatively to v14

**Files:**
- Create: `src-tauri/src/usage/cursor_migration.rs`
- Create: `src-tauri/src/usage/source_roots.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/tests.rs`
- Modify: `src-tauri/src/database/identity_migration.rs`

**Interfaces:**
- Produces `migrate_v13_to_v14(conn, roots) -> Result<()>` and `classify_legacy_cursor(path, roots) -> &'static str`.
- Produces pure `UsageSourceRoots::resolve_for_migration(home, explicit_config_dirs)` with no database access; unknown/unresolvable paths classify as `legacy`.
- `Database::init_at` resolves these roots from home plus pre-database config overrides and passes them into `apply_schema_migrations_on_conn_with_roots`; tests inject explicit roots through the same production-internal path.

- [ ] **Step 1: Write failing migration tests**

Create a v13 fixture with Claude, Codex, Gemini, OpenCode, and unknown `session_log_sync` rows. Assert migration renames the old table to `session_log_sync_v13_archive`, copies rows with source classification, preserves line offsets, sets byte offset/state to zero/NULL, labels unknown rows `legacy`, and sets `user_version=14`. Also assert a forced SQL failure rolls back the table rename and version. Update the database-filename migration test fixture so it still constructs an explicit schema-v13 source after global `SCHEMA_VERSION` becomes 14; it must not call the full current migration chain or silently create a v14 source. Re-run the crash-residue filename migration test to prove a fixed-v13 old filename still becomes a supported new-name v14 database through normal startup migration.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib migration_v13_to_v14 -- --nocapture`

Expected: FAIL because schema 13 has no migration arm or cursor archive.

- [ ] **Step 3: Implement transactional migration**

```rust
pub fn classify_legacy_cursor(path: &str, roots: &UsageSourceRoots) -> &'static str {
    let path = Path::new(path);
    if path.starts_with(&roots.claude) { "claude" }
    else if path.starts_with(&roots.codex) { "codex" }
    else if path.starts_with(&roots.gemini) { "gemini" }
    else if path.starts_with(&roots.opencode) { "opencode" }
    else { "legacy" }
}
```

Do not create a nested `rusqlite::Transaction` and do not set or commit `user_version` here. The existing `apply_schema_migrations_on_conn` savepoint owns atomicity and is the only code that sets version 14 after this function succeeds. Within that savepoint: create the v14 table, `ALTER TABLE session_log_sync RENAME TO session_log_sync_v13_archive`, read every archive row, classify it using roots resolved from home/config before database initialization, insert conservative cursors, and call `migrate_app_owned_identity_v14(conn)` from the identity plan. If the archive table already exists but v14 is incomplete, return an explicit migration error rather than overwriting evidence. Any cursor or identity-migration failure is rolled back by the outer savepoint together with the table rename and version.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib migration_v13_to_v14 -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib database::tests -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib crash_residue_wal_is_preserved_in_independent_new_and_archive_snapshots -- --nocapture
```

Expected: migration/rollback and all earlier schema tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/cursor_migration.rs src-tauri/src/usage/source_roots.rs src-tauri/src/usage/mod.rs src-tauri/src/database/schema.rs src-tauri/src/database/mod.rs src-tauri/src/database/tests.rs
git commit -m "feat(usage): migrate session cursors to schema v14"
```

### Task 3: Make Claude append parsing byte-incremental

**Files:**
- Modify: `src-tauri/src/services/session_usage.rs`
- Modify: `src-tauri/src/usage/ingestion.rs`
- Modify: `src-tauri/src/database/dao/usage_sync_cursors.rs`
- Add tests in: `src-tauri/src/services/session_usage.rs`

**Interfaces:**
- Produces `ClaudeParserStateV1 { version, session_id, line_offset }` and a parser that returns records plus `UsageSyncCursor` without persisting it.

- [ ] **Step 1: Write failing append, truncation, and failure tests**

Use a temp JSONL with real Claude assistant records. First sync records the cursor; wrap the file reader with a counting reader, append one record, sync again, and assert bytes read are bounded by appended bytes plus one line buffer. Replace/truncate the file and assert replay imports the valid replacement. Force failure on the Nth record and assert neither any usage row in that batch nor the prior cursor changes. Append a partial JSON line and assert byte offset stops before it.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib session_usage::tests::v14_ -- --nocapture`

Expected: FAIL because the importer still reads from byte zero and writes `session_log_sync` directly.

- [ ] **Step 3: Implement parse-then-ingest cursor flow**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ClaudeParserStateV1 {
    version: u8,
    session_id: Option<String>,
    line_offset: i64,
}

struct ParsedFileBatch {
    records: Vec<ParsedSessionRecord>,
    next_cursor: UsageSyncCursor,
}
```

Validate identity/size/offset/state, seek to byte offset on fast path, otherwise replay. Track the byte position after each complete newline. Add a shared `ingest_session_batch_and_advance_cursor(records, next_cursor)` database API that writes every usage record and the cursor in one savepoint/transaction on the same connection. Any record/cursor failure rolls back the entire batch. Keep the v13 archive read-only.

- [ ] **Step 4: Verify GREEN**

Run the Step 2 command plus existing Claude session tests.

Expected: append, truncation, partial line, failure, and legacy replay tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/session_usage.rs src-tauri/src/usage/ingestion.rs src-tauri/src/database/dao/usage_sync_cursors.rs
git commit -m "perf(usage): resume Claude session parsing by byte cursor"
```

### Task 4: Persist Codex cumulative parser state and exact deltas

**Files:**
- Modify: `src-tauri/src/services/session_usage_codex.rs`
- Modify: `src-tauri/src/usage/ingestion.rs`
- Modify: `src-tauri/src/database/dao/usage_sync_cursors.rs`
- Add tests in: `src-tauri/src/services/session_usage_codex.rs`

**Interfaces:**
- Produces versioned `CodexParserStateV1` containing session ID, current model, previous totals, event index, and line offset.

- [ ] **Step 1: Write failing state-resume tests**

Create a real Codex JSONL with `session_meta`, `turn_context`, and cumulative `token_count`. Sync once, append a larger cumulative count, sync again, and assert the second imported delta equals new total minus persisted previous total while old bytes are not reread. Serialize/reload the cursor to simulate restart. Add tests for unsupported state version, file identity replacement, zero delta, and an Nth-record ingestion failure that leaves both usage rows and cursor unchanged.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib session_usage_codex::tests::v14_ -- --nocapture`

Expected: FAIL because `FileParseState` is rebuilt by replaying all lines.

- [ ] **Step 3: Implement versioned state resume**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CodexParserStateV1 {
    version: u8,
    session_id: Option<String>,
    current_model: String,
    prev_total: Option<CumulativeTokens>,
    event_index: u64,
    line_offset: i64,
}
```

Use the same fast-path validation and complete-line byte tracking as Claude. Unsupported or inconsistent state replays from zero but suppresses emission through the preserved line offset/dedup IDs. Commit the full usage batch and next cursor through the shared atomic API; any row or cursor failure leaves both unchanged.

- [ ] **Step 4: Verify GREEN**

Run the Step 2 command plus all existing Codex session tests.

Expected: exact delta, restart, replacement, unsupported state, zero and failure tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/session_usage_codex.rs src-tauri/src/usage/ingestion.rs src-tauri/src/database/dao/usage_sync_cursors.rs
git commit -m "perf(usage): persist Codex session parser state"
```

### Task 5: Move Gemini and OpenCode to the common cursor DAO

**Files:**
- Modify: `src-tauri/src/services/session_usage_gemini.rs`
- Modify: `src-tauri/src/services/session_usage_opencode.rs`
- Modify: `src-tauri/src/services/session_usage.rs` shared cursor helpers.
- Modify: `src-tauri/src/usage/ingestion.rs`
- Modify: `src-tauri/src/database/dao/usage_sync_cursors.rs`

**Interfaces:**
- Gemini uses mtime/size whole-document cursors; OpenCode uses database and per-session watermark cursor keys.

- [ ] **Step 1: Write failing compatibility tests**

Assert unchanged Gemini JSON is skipped, changed JSON replays once, and parse/Nth-record ingestion failure leaves both usage rows and cursor unchanged. For OpenCode, create a real source DB/WAL fixture with two sessions, update one message, and assert only the changed session imports while database/session cursors advance atomically only after every usage row succeeds.

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib session_usage_gemini -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib session_usage_opencode -- --nocapture
```

Expected: FAIL because both modules still access `session_log_sync` helpers.

- [ ] **Step 3: Use source-aware cursor keys**

Remove direct legacy helper calls. Gemini key is its canonical file path. OpenCode file key is `database`, and session key is `session:<id>` under source `opencode`. Use the shared batch-and-cursor transaction API; preserve current incomplete-usage retry and WAL mtime behavior.

- [ ] **Step 4: Verify GREEN**

Run focused Gemini/OpenCode tests and `rg -n 'session_log_sync' src-tauri/src/services/session_usage*.rs`.

Expected: tests pass; no live importer reads/writes the archived v13 table.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/session_usage.rs src-tauri/src/services/session_usage_gemini.rs src-tauri/src/services/session_usage_opencode.rs src-tauri/src/usage/ingestion.rs src-tauri/src/database/dao/usage_sync_cursors.rs
git commit -m "refactor(usage): unify session source cursors"
```

### Task 6: Implement the pure configurable scheduler state machine

**Files:**
- Create: `src-tauri/src/usage/watcher_state.rs`
- Modify: `src-tauri/src/usage/mod.rs`

**Interfaces:**
- Produces `SourceId`, `SourceScheduleState`, `WatcherSchedule`, `mark_dirty`, `begin_due(now)`, `finish(source, generation, schedule_epoch, now, result)`, and `set_interval(now, seconds)`.
- `WatcherSchedule::default()` is exactly 300 seconds; accepted settings are 60-86400 seconds.

- [ ] **Step 1: Write failing deterministic state tests**

```rust
#[test]
fn event_storm_coalesces_and_never_starts_twice_inside_five_minutes() {
    let mut schedule = WatcherSchedule::new(300);
    for _ in 0..100 { schedule.mark_dirty(SourceId::Codex); }
    assert_eq!(schedule.begin_due(0), vec![(SourceId::Codex, 100)]);
    schedule.mark_dirty(SourceId::Codex);
    schedule.finish(SourceId::Codex, 100, 0, Ok(()));
    assert!(schedule.begin_due(299).is_empty());
    assert_eq!(schedule.begin_due(300), vec![(SourceId::Codex, 101)]);
}
```

Add tests for event-during-sync generation preservation, stale completion, failure backoff 5/10/20/30 minutes, success reset, overflow dirty-all, and idempotent shutdown. Add a test where `set_interval` runs during an in-flight sync and the older completion cannot overwrite the new epoch's re-armed deadline.
Add tests that default is 300 seconds, custom 60/600/3600-second windows gate correctly, a 60-second interval backs off exactly 60/120/240/360 seconds, invalid values normalize to 300 seconds, and changing the interval resets the next deadline from `now` without making a clean source dirty or launching immediately.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib watcher_state -- --nocapture`

Expected: FAIL because the state machine does not exist.

- [ ] **Step 3: Implement without wall-clock or watcher dependencies**

Use a monotonic `u64` seconds value supplied by the caller; the state machine must not read wall clock. Store configured interval, schedule epoch, dirty generation, syncing generation/epoch, next allowed second, consecutive failures, and shutdown flag per source. `mark_dirty` only increments generation. `begin_due` returns `(SourceId, generation, schedule_epoch)` and transitions eligible Dirty to Syncing once. `finish` clears only the generation it started; newer generation remains Dirty, and a completion from an older schedule epoch cannot replace a deadline set by `set_interval`. Failure delays use the configured interval multiplied by `[1, 2, 4, 6]`, indexed by `consecutive_failures.saturating_sub(1).min(3)`; the five-minute default therefore yields exactly 5/10/20/30 minutes. Success resets failures and enforces the configured window. `set_interval` validates 60-86400 seconds, increments the epoch, and re-arms deadlines from `now` without immediate work.

- [ ] **Step 4: Verify GREEN**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib watcher_state -- --nocapture`

Expected: all deterministic tests pass without sleeps.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/watcher_state.rs src-tauri/src/usage/mod.rs
git commit -m "feat(usage): add configurable watcher state machine"
```

### Task 7: Add native notify filtering, lifecycle, and blocking sync integration

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`
- Create: `src-tauri/src/usage/watcher.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/usage/session.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/commands/usage.rs`
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src/types.ts`
- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/usageDashboard.ts`
- Modify: `src/lib/query/usageDashboard.ts`
- Modify: `src/lib/schemas/settings.ts`
- Modify: `src/components/usage-dashboard/DataSourceBar.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.test.tsx`
- Modify: `src/i18n/locales/en.json`, `src/i18n/locales/ja.json`, `src/i18n/locales/zh-TW.json`, `src/i18n/locales/zh.json`
- Test: `src-tauri/tests/usage_dashboard_commands.rs`

**Interfaces:**
- Produces `SessionWatcherService`, `SessionWatcherHandle`, `WatcherDiagnosticView`; manual sync becomes async and shares the same per-source gate.
- Adds persisted `usage_session_sync_interval_minutes` (default 5, valid 1-1440) and a Dashboard selector for 1/5/10/15/30/60 minutes.

- [ ] **Step 1: Write failing path filter, overflow, lifecycle, and blocking tests**

Use temp external source/app/repo/cache roots. Assert only allowed extensions/names lexically map to a source. Put symlink/case aliases to excluded roots under an allowed source and assert the worker-side identity guard rejects them before parsing. Feed 1000 events into a small bounded channel and assert one `dirty_all`; `need_rescan()` has the same result. Assert dropping/stopping twice succeeds. Use a real slow closure on `spawn_blocking` and prove an async heartbeat remains responsive, plus a service-wide one-permit semaphore test proving four dirty startup sources do not parse concurrently. Add fake-clock tests for UI-ready startup reconciliation, watcher-backend initialization failure with reconciliation-only degradation, a missing root that never broadens to HOME and is retried later, persisted default/custom interval, and hot interval update without a second watcher or immediate run. Cover both manual provider sync and sync-all through the same gates. Do not assert mock calls.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage::watcher -- --nocapture`

Expected: FAIL because native watcher/lifecycle/filter APIs are absent.

- [ ] **Step 3: Add notify 8.2.0 and implement the callback boundary**

Add `notify = "8.2.0"`. Canonicalize and validate allow/deny roots when installing or updating recursive source watches; application/repository/cache roots are never registered. A missing source root remains reconciliation-only and is retried on later configured ticks; never fall back to watching HOME. The `recommended_watcher` callback converts `need_rescan()` and backend errors to `dirty_all`, filters `Access` events, performs only lexical root/name/extension classification and source deduplication, `try_send`s `SourceId`, sets `dirty_all` on a full channel, and returns. It performs no filesystem, database, sync I/O, or repeated per-event logging and holds no DB/service lock. Before a worker opens a source, canonicalize the resource and compare path plus macOS object identity against the deny set; rejection records a diagnostic and does not advance a cursor.

- [ ] **Step 4: Implement scheduler and app lifecycle**

Delete the existing independent 60-second session loop. After app state and an explicit UI-ready latch, mark all sources dirty and run one background reconciliation, then arm the next deadline at `now + configured_interval`. Use `interval_at` with the persisted interval and `MissedTickBehavior::Skip`; each due source enters its per-source gate and a service-wide one-permit semaphore before spawning blocking work, then reports completion through generation plus schedule epoch. If `recommended_watcher` construction fails, keep the scheduler in reconciliation-only mode and expose a degraded diagnostic instead of disabling sync. Add a source-scoped cursor metadata listing/reconciliation API; run reconciliation in `spawn_blocking`, then mark only changed sources dirty for that same gated tick. Treat a nominally successful parser result containing errors as failure/backoff. Both manual provider sync and sync-all use `try_begin_manual` in fixed source order, return a visible busy result without partial work, and reset the configured window after success. Keep `usage_session_sync_interval_minutes` independent from frontend query refresh. Settings save validates integer 1-1440 minutes and sends acknowledged `SetInterval`/`UpdateRoots` commands through the existing serialized watcher channel; persistence must roll back or fail if runtime acknowledgement fails. It re-arms from the change time without immediate sync or watcher recreation. Provider source-binding save/enable changes send an acknowledged `RefreshActiveSources`; the owner only watch/unwatches roots on its one existing native watcher. `AppState` owns one optional handle. Shutdown marks state stopped, cancels interval, closes channels, unwatches roots, awaits every blocking job, then drops the watcher; repeated stop is idempotent.

- [ ] **Step 5: Verify GREEN including one real watcher smoke**

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage::watcher -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage::session -- --nocapture
```

Expected: deterministic tests pass; one temp directory create/append event marks the source dirty without immediate sync; excluded app-data write produces no source event; default UI/settings value is 5 minutes and a 10-minute change re-arms one existing scheduler.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/usage src-tauri/src/store.rs src-tauri/src/lib.rs src-tauri/src/commands/usage.rs src-tauri/src/commands/usage_dashboard.rs src-tauri/src/commands/settings.rs src-tauri/src/settings.rs src-tauri/tests/usage_dashboard_commands.rs src/types.ts src/types/usageDashboard.ts src/lib/api/usageDashboard.ts src/lib/query/usageDashboard.ts src/lib/schemas/settings.ts src/components/usage-dashboard/DataSourceBar.tsx src/components/usage-dashboard/UsageDashboardPage.tsx src/components/usage-dashboard/UsageDashboardPage.test.tsx src/i18n/locales
git commit -m "feat(usage): add configurable filesystem watcher"
```

### Task 8: Run full regression, loop-safety, and desktop smoke gates

**Files:**
- Modify only files required by failures found in this task.

**Interfaces:**
- Produces the combined v14/watcher result ready for whole-branch review.

- [ ] **Step 1: Run focused schema/parser/watcher gates**

```bash
pnpm rust -- fmt --check --manifest-path src-tauri/Cargo.toml
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage_sync_cursor -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib session_usage -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib usage::watcher -- --nocapture
```

Expected: all pass with no warnings.

- [ ] **Step 2: Run complete automated gates**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
git diff --check
test ! -d src-tauri/target
```

Expected: all exit 0 and Rust artifacts exist only in the shared lock-hash bucket.

- [ ] **Step 3: Run isolated v13-to-v14 desktop smoke**

Use a temporary HOME with v13 current-app database and temporary Claude/Codex/Gemini/OpenCode sources. Launch with `pnpm tauri -- dev`. Verify migration to v14, watcher diagnostic roots exclude both app-data directories/repository/cache, the default is five minutes, a safe custom interval hot-updates the same watcher, event storms do not cause more than one sync inside the configured window, manual sync reports busy during an active source sync, Dashboard remains responsive, shutdown is clean, and no `crash.log` appears.

- [ ] **Step 4: Commit verification fixes only if needed**

```bash
git add src-tauri src tests
git commit -m "fix(usage): close v14 watcher regressions"
```

If no fix is needed, do not create an empty commit.
