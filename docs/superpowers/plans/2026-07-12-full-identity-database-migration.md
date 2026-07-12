# Full Product Identity and Database Filename Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every LLM Usage Bar-owned manifest, executable, library, log, database filename, internal discriminator, visible string, package, and release surface use the new identity while preserving explicit read-only Legacy CC Switch compatibility.

**Architecture:** A typed Rust identity module separates current and legacy constants. A pre-open SQLite Backup API migration safely publishes `llm-usage-bar.db` from the current app's old filename. A cross-language identity contract test and explicit legacy allowlist prevent both incomplete renames and destructive global replacement.

**Tech Stack:** Rust 1.95, rusqlite Backup API, Tauri 2, TypeScript/Vitest, pnpm, Flatpak/WiX metadata.

## Global Constraints

- Run Rust commands through `pnpm rust -- ...`; do not recreate `src-tauri/target`.
- Current identity is `LLM Usage Bar`, slug/package `llm-usage-bar`, Rust library `llm_usage_bar_lib`, bundle `com.llmusagebar.desktop`, database `llm-usage-bar.db`, log basename `llm-usage-bar`, repository `Xr810/LLM-Usage-Bar`, port `15722`.
- Never write, rename, migrate, or open read-write under real `~/.cc-switch`.
- Only `~/.llm-usage-bar/cc-switch.db` is an automatic filename-migration source.
- Never copy a live SQLite file with `fs::copy`; use SQLite Backup API, validate, and atomically publish.
- New database filename wins if both old and new exist.
- Preserve protocol/provider IDs unless they are app-owned current-product discriminators.
- Original-product values are relabeled only as explicit `legacy-cc-switch` where storage needs disambiguation.
- Historical plan/task-state documents remain unchanged.
- Do not point the app at the public CC Switch updater; if no reachable LLM Usage Bar feed exists, report managed updates unavailable.

---

### Task 1: Establish a typed current-versus-legacy identity contract

**Files:**
- Create: `src-tauri/src/product_identity.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `package.json`
- Create: `tests/config/productIdentity.test.ts`

**Interfaces:**
- Produces current constants `DISPLAY_NAME`, `APP_SLUG`, `DATABASE_FILE`, `LOG_BASENAME`, `BUNDLE_ID`, `REPOSITORY`.
- Produces legacy constants `LEGACY_DISPLAY_NAME`, `LEGACY_SLUG`, `LEGACY_DATA_DIR`, `LEGACY_DATABASE_FILE`.

- [ ] **Step 1: Write failing manifest and Rust identity tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_and_legacy_identity_are_explicitly_separate() {
        assert_eq!(DISPLAY_NAME, "LLM Usage Bar");
        assert_eq!(APP_SLUG, "llm-usage-bar");
        assert_eq!(DATABASE_FILE, "llm-usage-bar.db");
        assert_eq!(LOG_BASENAME, "llm-usage-bar");
        assert_eq!(LEGACY_DISPLAY_NAME, "CC Switch");
        assert_eq!(LEGACY_SLUG, "cc-switch");
        assert_ne!(APP_SLUG, LEGACY_SLUG);
    }
}
```

```ts
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parse as parseToml } from "smol-toml";

describe("product identity manifests", () => {
  it("uses the LLM Usage Bar package and bundle identity", () => {
    const pkg = JSON.parse(readFileSync("package.json", "utf8"));
    const cargo = parseToml(readFileSync("src-tauri/Cargo.toml", "utf8"));
    const tauri = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    expect(pkg.name).toBe("llm-usage-bar");
    expect(cargo.package.name).toBe("llm-usage-bar");
    expect(cargo.lib.name).toBe("llm_usage_bar_lib");
    expect(tauri.productName).toBe("LLM Usage Bar");
    expect(tauri.identifier).toBe("com.llmusagebar.desktop");
  });
});
```

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm test:unit tests/config/productIdentity.test.ts
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib product_identity -- --nocapture
```

Expected: frontend assertion sees `cc-switch`; the focused Rust library test fails because `product_identity` does not exist. The `--lib` selector is intentional: integration tests still import the legacy crate name until Task 5 completes the repository-wide product identity migration.

- [ ] **Step 3: Add identity constants and rename manifests**

```rust
use std::path::PathBuf;

pub const DISPLAY_NAME: &str = "LLM Usage Bar";
pub const APP_SLUG: &str = "llm-usage-bar";
pub const DATABASE_FILE: &str = "llm-usage-bar.db";
pub const LOG_BASENAME: &str = "llm-usage-bar";
pub const BUNDLE_ID: &str = "com.llmusagebar.desktop";
pub const REPOSITORY: &str = "Xr810/LLM-Usage-Bar";
pub const LEGACY_DISPLAY_NAME: &str = "CC Switch";
pub const LEGACY_SLUG: &str = "cc-switch";
pub const LEGACY_DATA_DIR: &str = ".cc-switch";
pub const LEGACY_DATABASE_FILE: &str = "cc-switch.db";

pub fn current_database_path(app_dir: &std::path::Path) -> PathBuf {
    app_dir.join(DATABASE_FILE)
}

pub fn prior_current_app_database_path(app_dir: &std::path::Path) -> PathBuf {
    app_dir.join(LEGACY_DATABASE_FILE)
}
```

Set Cargo package to `llm-usage-bar`, library to `llm_usage_bar_lib`, repository to `https://github.com/Xr810/LLM-Usage-Bar`, and npm package to `llm-usage-bar`. Update `main.rs` to call `llm_usage_bar_lib::run()`. Regenerate only the root package entry in `Cargo.lock` with `pnpm rust -- check --manifest-path src-tauri/Cargo.toml --lib`.

- [ ] **Step 4: Verify GREEN**

Run the two Step 2 commands.

Expected: identity tests pass; the compiled package is `llm-usage-bar` and no worktree-local target appears.

- [ ] **Step 5: Commit**

```bash
git add package.json tests/config/productIdentity.test.ts src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs src-tauri/src/lib.rs src-tauri/src/product_identity.rs
git commit -m "refactor(identity): establish LLM Usage Bar namespace"
```

### Task 2: Implement safe current-app database filename migration

**Files:**
- Create: `src-tauri/src/database/identity_migration.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/product_identity.rs`
- Test: `src-tauri/src/database/identity_migration.rs`

**Interfaces:**
- Produces `prepare_database_identity(app_dir: &Path) -> Result<DatabaseIdentityOutcome, AppError>` under one exclusive per-directory migration lease.
- `DatabaseIdentityOutcome` includes authoritative path and migration/archive state.

- [ ] **Step 1: Write failing real-SQLite tests**

```rust
#[test]
fn backup_captures_committed_uncheckpointed_wal() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("cc-switch.db");
    let temp = dir.path().join("backup-test.db");
    let writer = create_real_v13_fixture(&old);
    writer.pragma_update(None, "journal_mode", "WAL").unwrap();
    writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    writer.execute_batch("CREATE TABLE marker(value TEXT); PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    let reader = rusqlite::Connection::open(&old).unwrap();
    reader.execute_batch("BEGIN; SELECT * FROM marker;").unwrap();
    writer.execute("INSERT INTO marker VALUES ('from-uncheckpointed-wal')", []).unwrap();
    assert!(std::fs::metadata(old.with_extension("db-wal")).unwrap().len() > 32);
    backup_and_validate(&old, &temp).unwrap();
    assert_eq!(read_marker(&temp), "from-uncheckpointed-wal");
    drop(reader);
    drop(writer);
}

#[test]
fn migrates_prior_current_app_database_and_archives_source() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("cc-switch.db");
    let conn = create_real_v13_fixture(&old);
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch("CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('from-wal');").unwrap();
    drop(conn);
    let outcome = prepare_database_identity(dir.path()).unwrap();
    assert_eq!(outcome.database_path, dir.path().join("llm-usage-bar.db"));
    let migrated = rusqlite::Connection::open(outcome.database_path).unwrap();
    assert_eq!(migrated.query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0)).unwrap(), "from-wal");
    assert!(dir.path().join("cc-switch.db.pre-llm-usage-bar-v14").exists());
}

#[test]
fn new_filename_wins_when_both_files_exist() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("cc-switch.db");
    let new = dir.path().join("llm-usage-bar.db");
    write_marker_db(&old, "old");
    write_marker_db(&new, "new");
    let old_before = std::fs::read(&old).unwrap();
    let outcome = prepare_database_identity(dir.path()).unwrap();
    assert_eq!(outcome.database_path, new);
    assert!(!outcome.migrated);
    assert_eq!(read_marker(&outcome.database_path), "new");
    assert_eq!(std::fs::read(&old).unwrap(), old_before);
    assert!(!dir.path().join("cc-switch.db.pre-llm-usage-bar-v14").exists());
}

#[test]
fn failed_validation_leaves_source_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("cc-switch.db");
    std::fs::write(&old, b"not sqlite").unwrap();
    let before = std::fs::read(&old).unwrap();
    assert!(prepare_database_identity(dir.path()).is_err());
    assert_eq!(std::fs::read(&old).unwrap(), before);
    assert!(!dir.path().join("llm-usage-bar.db").exists());
}
```

The WAL test keeps writer and reader transactions open, asserts a nonempty `-wal` contains the committed row before backup, and verifies the backup before either connection closes; it therefore cannot pass through an automatic last-connection checkpoint. Add a subprocess fixture that commits a real-v13 WAL row and calls `process::exit` before SQLite destructors run; after migration, both the new database and the archive must contain that row, must have different filesystem object identities, and mutating the new database must not alter the archive. The separate ordinary end-to-end migration test closes the fixture writer before archive/publish assertions for cross-platform behavior. `backup_and_validate` must enumerate the required v13 tables used by the application, so a marker-only database is intentionally invalid. Add tests for quick-check failure, future/unsupported `user_version`, an existing archive path, two concurrent callers, a forced archive failure after publish, fail-closed live/malformed/stale lease files, and an app-dir/old-path alias to real `~/.cc-switch` that leaves the forbidden directory entry list and bytes unchanged.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib identity_migration -- --nocapture`

Expected: FAIL because the module and API are absent.

- [ ] **Step 3: Implement Backup API, validation, publish, and archive**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseIdentityOutcome {
    pub database_path: PathBuf,
    pub archived_prior_path: Option<PathBuf>,
    pub retained_prior_path: Option<PathBuf>,
    pub migrated: bool,
}

pub fn prepare_database_identity(app_dir: &Path) -> Result<DatabaseIdentityOutcome, AppError> {
    reject_legacy_namespace_before_any_io(app_dir)?;
    let new_path = app_dir.join(DATABASE_FILE);
    let old_path = app_dir.join(LEGACY_DATABASE_FILE);
    let archive_path = app_dir.join("cc-switch.db.pre-llm-usage-bar-v14");
    reject_legacy_namespace_candidates(app_dir, [&old_path, &new_path, &archive_path])?;
    let _lease = DatabaseMigrationLease::acquire(app_dir)?;
    if new_path.exists() {
        return Ok(DatabaseIdentityOutcome { database_path: new_path, archived_prior_path: None, retained_prior_path: old_path.exists().then_some(old_path), migrated: false });
    }
    if !old_path.exists() {
        return Ok(DatabaseIdentityOutcome { database_path: new_path, archived_prior_path: None, retained_prior_path: None, migrated: false });
    }
    let temp_path = unique_same_directory_temp(app_dir)?;
    reject_legacy_namespace_candidates(app_dir, [&temp_path])?;
    fail_if_archive_already_exists(&archive_path)?;
    backup_and_validate(&old_path, &temp_path)?;
    let archive_temp = unique_same_directory_temp(app_dir)?;
    reject_legacy_namespace_candidates(app_dir, [&archive_temp])?;
    materialize_independent_validated_snapshot(&temp_path, &archive_temp)?;
    if publish_noclobber(&temp_path, &new_path)? == PublishOutcome::ExistingWon {
        return Ok(DatabaseIdentityOutcome { database_path: new_path, archived_prior_path: None, retained_prior_path: Some(old_path), migrated: false });
    }
    if let Err(error) = publish_archive_and_remove_prior(&archive_temp, &archive_path, &old_path) {
        rollback_only_this_invocation(&temp_path, &new_path, &archive_temp, &archive_path)?;
        return Err(error);
    }
    Ok(DatabaseIdentityOutcome { database_path: new_path, archived_prior_path: Some(archive_path), retained_prior_path: None, migrated: true })
}
```

`reject_legacy_namespace_before_any_io` and the candidate guard perform lexical normalization, canonical comparison where paths exist, and Unix device/inode checks before any `mkdir`, lease creation, file open, or authoritative-path early return. An app-dir alias to real `~/.cc-switch` must leave the directory entry list and bytes completely unchanged. `DatabaseMigrationLease` is an RAII `create_new` lock with owner metadata and bounded waiting. Lock recovery is fail-closed in this task: live, malformed, partial, indeterminate, age-old, and even apparently dead-owner lock files are never unlinked automatically because a PID probe plus path recheck cannot eliminate the read/probe/remove ABA race with only the current standard-library file scope. The returned error names the lock path for deliberate operator recovery. `backup_and_validate` opens source read-only, creates a unique same-directory destination, runs `rusqlite::backup::Backup::new`, completes to `StepResult::Done`, executes `PRAGMA quick_check`, verifies required schema/user version, flushes, and removes only its temporary destination on error. `publish_noclobber` uses an atomic same-directory no-clobber publish; if another valid new database appears, discard only the temporary copy and return the new filename as authoritative. Never use `rename` semantics that can overwrite the new path. The archive is a second validated snapshot with a different filesystem identity from the new database, not a hard link of either the new snapshot or the old main file. This preserves committed crash-residue WAL rows and prevents later v14 writes from changing the archive. Archive publication and old-source removal use no-clobber semantics and never replace existing evidence. If either fails after this invocation published the new path, remove only this invocation's archive and new file after object-identity checks so the old-only state is restored; a rollback-cleanup failure is surfaced explicitly and never causes deletion of pre-existing evidence.

- [ ] **Step 4: Verify GREEN**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib identity_migration -- --nocapture`

Expected: real-schema WAL, both-exist/race, archive collision/rollback, concurrent-call, invalid/future source, original-data alias, and fresh-install tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/database/identity_migration.rs src-tauri/src/database/mod.rs src-tauri/src/product_identity.rs
git commit -m "feat(identity): migrate current app database filename safely"
```

### Task 3: Integrate the new database and log paths into startup

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/panic_hook.rs`
- Modify: `src-tauri/src/database/backup.rs`
- Test: `src-tauri/tests/app_config_load.rs`

**Interfaces:**
- Consumes `prepare_database_identity`; produces one authoritative `llm-usage-bar.db` path for preflight, `Database::init_at`, backup, recovery, and UI diagnostics.

- [ ] **Step 1: Add failing startup path tests**

Create an isolated HOME containing `.llm-usage-bar/cc-switch.db`, invoke the existing app-config/database initialization test hook, and assert every returned path uses `llm-usage-bar.db`, the file log is `logs/llm-usage-bar.log`, and panic log remains `crash.log`.

- [ ] **Step 2: Verify RED**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test app_config_load database_identity -- --nocapture`

Expected: FAIL because startup still joins `cc-switch.db` and logging still uses `cc-switch`.

- [ ] **Step 3: Thread the authoritative path through startup**

Call `prepare_database_identity` before `stored_user_version_exceeds_supported`; add `Database::init_at(path: &Path)` and keep `Database::init()` as a thin call using current identity. Replace current-product log/header names with identity constants. Do not modify paths inside the explicit original-data import feature.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --test app_config_load -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib database:: -- --nocapture
```

Expected: startup and database tests pass; migrated and fresh databases open through the new path.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/database/mod.rs src-tauri/src/panic_hook.rs src-tauri/src/database/backup.rs src-tauri/tests/app_config_load.rs
git commit -m "refactor(identity): use LLM Usage Bar runtime paths"
```

### Task 4: Migrate app-owned internal discriminators without corrupting legacy values

**Files:**
- Modify: `src-tauri/src/services/skill.rs`
- Modify: `src-tauri/src/app_config.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/tests.rs`
- Modify: `src/types.ts`
- Modify: `src/lib/api/skills.ts`
- Modify: `src/components/settings/SkillStorageLocationSettings.tsx`
- Create: `tests/config/productIdentityAllowlist.ts`

**Interfaces:**
- Skill storage wire value is exactly `llm_usage_bar`; legacy `cc_switch` is read as an alias, and `unified` remains unchanged. There is no writable Legacy CC Switch storage variant.
- Database provenance/current-product discriminators use exactly `llm-usage-bar`; original-product provenance uses `legacy-cc-switch` in a separate read-only origin type.
- Produces `migrate_app_owned_identity_v14(conn: &rusqlite::Connection) -> Result<()>`; the caller owns the surrounding savepoint/transaction and `user_version`.

- [ ] **Step 1: Write failing serialization and migration tests**

Test that the renamed storage-location enum serializes to `llm_usage_bar`, deserializes old `cc_switch` as an alias, and can write only the current app directory or unified standards directory. Build a v13 fixture, open one savepoint, call `migrate_app_owned_identity_v14(&conn)` directly, and assert rows whose meaning is current-app become `llm-usage-bar`, imported original rows become `legacy-cc-switch`, and unrelated provider/protocol IDs remain byte-for-byte unchanged. Roll the test savepoint back so this task does not bump `user_version`. Add a path test proving no decoded storage value resolves under real `~/.cc-switch`.

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib identity_discriminator -- --nocapture
pnpm typecheck
```

Expected: tests fail on old variant/value; typecheck identifies exact frontend consumers.

- [ ] **Step 3: Rename the writable current variant and add a legacy read alias**

Use serde aliases rather than string replacement:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillStorageLocation {
    #[serde(alias = "cc_switch")]
    LlmUsageBar,
    Unified,
}
```

Before changing code, commit an exact classification table in `tests/config/productIdentityAllowlist.ts`: each old literal is `ownedRename`, `legacyReadOnly`, or `externalWireStable`, with exact file and constant/key context. At minimum, remote roots/protocols `cc-switch-sync` and `cc-switch-webdav-sync`, existing Codex catalog/provider identifiers, OAuth compatibility headers, legacy SQL import header, old environment-variable aliases, and serialized error codes are wire-stable unless a dual-read/dual-write migration is separately tested. Add byte-stability tests in their owning Rust/TypeScript modules. Add `migrate_app_owned_identity_v14` with explicit SQL updates only for enumerated columns/settings keys. Decode JSON settings into typed values before rewriting. Do not update protocol/provider IDs globally. Do not call the helper from schema 13; the watcher plan invokes it inside the existing schema savepoint.

- [ ] **Step 4: Verify GREEN**

Run the Step 2 commands plus focused provider/config tests.

Expected: identity tests and typecheck pass; approved legacy values remain.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/skill.rs src-tauri/src/app_config.rs src-tauri/src/database/schema.rs src-tauri/src/database/tests.rs src
git commit -m "refactor(identity): migrate app-owned internal identifiers"
```

### Task 5: Replace current-product UI, updater, packaging, and repository identity

**Files:**
- Modify: `src/i18n/locales/en.json`, `src/i18n/locales/ja.json`, `src/i18n/locales/zh-TW.json`, `src/i18n/locales/zh.json`
- Modify: current-product files enumerated by `runtimeAndCurrentDocs()` under `src/`; the expected typed/runtime set includes `src/types.ts`, `src/lib/api/skills.ts`, `src/lib/schemas/settings.ts`, `src/hooks/useProviderActions.ts`, `src/components/DatabaseUpgrade.tsx`, `src/components/settings/AboutSection.tsx`, `src/components/settings/DirectorySettings.tsx`, and `src/components/settings/SkillStorageLocationSettings.tsx`
- Modify: `README.md`, `README_ZH.md`, `README_DE.md`, `README_JA.md`, `CONTRIBUTING.md`, `SECURITY.md`, `SUPPORT.md`
- Modify: `.github/CODEOWNERS`, `.github/FUNDING.yml`, `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/ISSUE_TEMPLATE/doc_issue.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/ISSUE_TEMPLATE/question.yml`, `.github/workflows/release.yml`
- Modify: `src-tauri/tauri.conf.json`, `src-tauri/Info.plist`, `src-tauri/src/commands/misc.rs`, `src-tauri/src/auto_launch.rs`, `src-tauri/src/deeplink/mod.rs`, `src-tauri/wix/per-user-main.wxs`
- Rename/modify: `flatpak/com.ccswitch.desktop.desktop` to `flatpak/com.llmusagebar.desktop.desktop`
- Rename/modify: `flatpak/com.ccswitch.desktop.metainfo.xml` to `flatpak/com.llmusagebar.desktop.metainfo.xml`
- Rename/modify: `flatpak/com.ccswitch.desktop.yml` to `flatpak/com.llmusagebar.desktop.yml`
- Modify: `flatpak/README.md`
- Modify: `tests/config/productIdentity.test.ts`

**Interfaces:**
- Produces a visible/current identity audit plus exact owned/legacy/wire-stable classification; unclassified occurrences fail.

- [ ] **Step 1: Expand the identity test to fail on unapproved old identity**

```ts
import { classifyOldIdentityOccurrence } from "./productIdentityAllowlist";

it("classifies every old identity occurrence and leaves no owned rename", () => {
  for (const file of runtimeAndCurrentDocs()) {
    for (const [index, line] of readFileSync(file, "utf8").split("\n").entries()) {
      if (/CC Switch|cc-switch|cc_switch/.test(line)) {
        const classification = classifyOldIdentityOccurrence(file, index + 1, line);
        expect(classification, `${file}:${index + 1} ${line}`).toBeDefined();
        expect(classification?.kind, `${file}:${index + 1} ${line}`).not.toBe("ownedRename");
      }
    }
  }
});
```

`runtimeAndCurrentDocs()` must enumerate `src/**`, `src-tauri/src/**`, `src-tauri/tests/**`, locales, manifests, packaging, and current root docs. It excludes historical `docs/superpowers/plans`, `docs/task-state`, generated output, and protocol literals only through exact entries from `productIdentityAllowlist.ts`; regex-only blanket exemptions are forbidden.

- [ ] **Step 2: Verify RED and capture the exact inventory**

Run: `pnpm test:unit tests/config/productIdentity.test.ts`

Expected: FAIL listing every remaining unapproved current-product identity occurrence.

- [ ] **Step 3: Replace owned identity and label preserved compatibility**

Classify every failure first. Rename only `ownedRename` entries, label `legacyReadOnly` entries, and keep `externalWireStable` bytes unchanged with focused contract tests. Old SQL export headers remain accepted on import, while new exports use `LLM Usage Bar SQLite export`. Replace upstream update checks with a deterministic `managed updates unavailable` response until a reachable LLM Usage Bar endpoint exists. Rename Flatpak/app IDs and auto-start labels. Keep partner coupon codes and inherited issue URLs only where they are external literals, and label explanatory UI as legacy.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm test:unit tests/config/productIdentity.test.ts
pnpm typecheck
pnpm format:check
pnpm build:renderer
git diff --check
```

Expected: identity audit and all frontend gates pass.

- [ ] **Step 5: Commit**

```bash
git add src src-tauri flatpak README*.md CONTRIBUTING.md SECURITY.md SUPPORT.md .github tests/config/productIdentity.test.ts tests/config/productIdentityAllowlist.ts
git commit -m "refactor(identity): complete LLM Usage Bar product migration"
```

### Task 6: Complete identity regression and isolated migration smoke

**Files:**
- Modify only files required by failures found in this task.

**Interfaces:**
- Produces a clean intermediate branch that schema-v14 work can build on.

- [ ] **Step 1: Run focused identity/database gates**

```bash
pnpm rust -- fmt --check --manifest-path src-tauri/Cargo.toml
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib identity -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib database:: -- --nocapture
pnpm test:unit tests/config/productIdentity.test.ts
```

Expected: all exit 0.

- [ ] **Step 2: Run full automated gates**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
git diff --check
```

Expected: all Rust and frontend tests pass with no warnings/errors.

- [ ] **Step 3: Run an isolated-HOME desktop filename migration smoke**

Create a temporary HOME containing a v13 `.llm-usage-bar/cc-switch.db` fixture and a separate sentinel under `.cc-switch`. Launch with `pnpm tauri -- dev`, verify process/bundle name `llm-usage-bar`, new database/log filenames, migrated rows, old archive, and unchanged sentinel; then stop with Ctrl-C.

- [ ] **Step 4: Commit verification fixes only if needed**

```bash
git add package.json src src-tauri flatpak README*.md CONTRIBUTING.md SECURITY.md SUPPORT.md .github tests
git commit -m "fix(identity): close full migration regressions"
```

If no fix is needed, do not create an empty commit.
