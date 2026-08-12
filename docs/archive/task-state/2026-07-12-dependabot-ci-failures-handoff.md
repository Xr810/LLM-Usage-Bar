# Dependabot CI Failures Handoff

Last verified: 2026-07-12 09:46 Asia/Singapore

Repository: `/Users/max/LLM Usage Bar`

Remote: `Xr810/LLM-Usage-Bar`

Status: **diagnosed, not fixed**. This document is a handoff for a separate repair task.

## Executive summary

The red checks visible in the pull request list do not belong to usage-dashboard PR #6. PR #6 is green. The remaining red checks belong to two independent Dependabot pull requests created from the old `main` base `57305f5b`:

| PR | Failing check | Immediate cause |
| --- | --- | --- |
| [#4 frontend-deps](https://github.com/Xr810/LLM-Usage-Bar/pull/4) | Frontend Checks / TypeScript type check | TypeScript 7 rejects the existing `baseUrl` and non-relative `paths` configuration |
| [#5 cargo-deps](https://github.com/Xr810/LLM-Usage-Bar/pull/5) | Backend Checks / Clippy compilation | Major `rusqlite`, `hmac`, and `sha2` upgrades break existing database and crypto APIs |

Both PRs group every available dependency update, including major versions. The current `.github/dependabot.yml` uses `patterns: ["*"]` without restricting `update-types`, so major, minor, and patch updates are placed in one large PR per ecosystem.

## PR #4: frontend dependency group

### Evidence and check state

- PR: [#4](https://github.com/Xr810/LLM-Usage-Bar/pull/4)
- Head: `c3f15c78b498bfe357c042c19d5c32fbf1e6fe34`
- Base used by the failing run: `57305f5b2db57e5ff9d16e49ad080d18bed7f6b3`
- Failing job: [CI / Frontend Checks](https://github.com/Xr810/LLM-Usage-Bar/actions/runs/29142469849/job/86518107402)
- Install dependencies: passed.
- TypeScript type check: failed.
- Formatting and unit tests: skipped because the job stopped at typecheck.
- Backend Checks and PR labelling: passed.

### Dependency change that triggers the first failure

Dependabot changes TypeScript from the repository's resolved 5.9.2 line to 7.0.2:

```diff
- "typescript": "^5.3.0"
+ "typescript": "^7.0.2"
```

The same grouped PR also moves React 18 to 19, Tailwind 3 to 4, Vite 7 to 8, Vitest 2 to 4, and many other packages. It changes 54 direct dependency declarations plus the lockfile.

### Exact errors

```text
tsconfig.json(17,5): error TS5102: Option 'baseUrl' has been removed.
Use '"paths": {"*": ["./*"]}' instead.

tsconfig.json(19,15): error TS5090: Non-relative paths are not allowed.
Did you forget a leading './'?
```

The existing configuration is effectively:

```json
{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["src/*"]
    }
  }
}
```

TypeScript 7 removed `baseUrl`; path mappings must be relative to the project root. The forward migration is to remove `baseUrl` and change the mapping to `"@/*": ["./src/*"]`.

### Root-cause chain

```text
Dependabot wildcard group permits major updates
  -> TypeScript 5.9.2 becomes 7.0.2
  -> TypeScript 7 reads unchanged tsconfig.json
  -> removed baseUrl produces TS5102
  -> src/* is no longer prefixed by baseUrl and is non-relative
  -> TS5090
  -> CI stops before formatting and unit tests
```

### Repair choices

Safest short-term choice:

1. Remove TypeScript 7 from this grouped update or pin TypeScript to the current 5.x line.
2. Regenerate `pnpm-lock.yaml`.
3. Run the complete frontend gate.
4. Upgrade TypeScript, React, Tailwind, Vite, and Vitest in separate major-version PRs.

If the task explicitly chooses a TypeScript 7 migration instead:

1. Delete `baseUrl` from `tsconfig.json`.
2. Change `"@/*": ["src/*"]` to `"@/*": ["./src/*"]`.
3. Run the complete frontend gate; do not stop after typecheck because the other 53 updates have not yet been exercised.

Required verification:

```bash
pnpm install --frozen-lockfile
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
```

## PR #5: Cargo dependency group

### Evidence and check state

- PR: [#5](https://github.com/Xr810/LLM-Usage-Bar/pull/5)
- Head: `2792e538ab778c07d8b643c2c3a6ded61e995bfd`
- Base used by the failing run: `57305f5b2db57e5ff9d16e49ad080d18bed7f6b3`
- Failing job: [CI / Backend Checks](https://github.com/Xr810/LLM-Usage-Bar/actions/runs/29142612771/job/86518486551)
- Rust formatting: passed.
- Clippy compilation: failed with 12 compiler errors.
- Rust tests: skipped because Clippy failed.
- Frontend Checks and PR labelling: passed.

### Error group 1: rusqlite 0.31 to 0.40

Dependabot changes:

```diff
- rusqlite = { version = "0.31", features = ["bundled", "backup", "hooks"] }
+ rusqlite = { version = "0.40", features = ["bundled", "backup", "hooks"] }
```

Resolved failing version: `rusqlite 0.40.1`.

Six `E0277` errors occur because `usize` no longer implements `FromSql`/`ToSql` under the selected default features. In the newer rusqlite line, unsigned integer support is gated behind the `fallible_uint` feature.

`usize: FromSql` failures:

- `src-tauri/src/database/dao/failover.rs:40`
- `src-tauri/src/database/dao/providers.rs:39`
- `src-tauri/src/database/dao/providers.rs:146`
- `src-tauri/src/database/dao/providers.rs:460`

`usize: ToSql` failures:

- `src-tauri/src/database/dao/providers.rs:217`
- `src-tauri/src/database/migration.rs:88`

Minimal forward-compatible option:

```toml
rusqlite = {
  version = "0.40",
  features = ["bundled", "backup", "hooks", "fallible_uint"]
}
```

This preserves the existing `Option<usize>` API and fails conversion when a value exceeds SQLite's signed `i64` range. A more explicit but broader alternative is to use `i64` at every SQLite boundary and checked conversions at the domain boundary.

### Error group 2: hmac 0.12 to 0.13

Dependabot changes `hmac = "0.12"` to `hmac = "0.13"`.

Two `E0599` errors:

- `src-tauri/src/services/coding_plan.rs:805`
- `src-tauri/src/services/s3.rs:95`

Both fail at:

```rust
HmacSha256::new_from_slice(key)
```

In hmac 0.13, `new_from_slice` is supplied by `KeyInit`. The minimal source migration is to bring that trait into scope in both local helper functions:

```rust
use hmac::{Hmac, KeyInit, Mac};
```

### Error group 3: sha2 0.10 to 0.11

Dependabot changes `sha2 = "0.10"` to `sha2 = "0.11"`.

Four `E0277` errors:

- `src-tauri/src/services/coding_plan.rs:812`
- `src-tauri/src/services/s3.rs:102`
- `src-tauri/src/services/skill.rs:849`
- `src-tauri/src/services/sync_protocol.rs:347`

All four use this old formatting shape:

```rust
format!("{:x}", hasher.finalize())
```

sha2 0.11 returns a new array type that does not implement `LowerHex`. A minimal semantic-preserving migration is to add `hex = "0.4"` and use `hex::encode(...)`, which continues to produce lowercase hexadecimal strings.

### Root-cause chain

```text
Dependabot wildcard group permits 42 Cargo updates
  -> several API-breaking major versions move together
  -> rusqlite unsigned SQL conversions require a new feature
  -> hmac constructor requires KeyInit in scope
  -> sha2 digest output no longer supports LowerHex formatting
  -> Clippy cannot compile the library (12 errors)
  -> Rust tests are skipped
```

### Required verification after repair

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Fixing the current 12 errors does not prove the other Cargo upgrades are compatible. The full test step has never run for PR #5, and other upgraded packages include major changes to Axum, Tower, TOML, QuickJS, Zip, JSON5, and platform libraries.

## Why the grouped PRs were created

Current configuration:

```yaml
groups:
  frontend-deps:
    patterns:
      - "*"
```

and the equivalent Cargo group match every dependency. Because neither group limits `update-types`, Dependabot includes major, minor, and patch updates together.

Recommended future configuration shape:

```yaml
groups:
  frontend-minor-patch:
    patterns:
      - "*"
    update-types:
      - "minor"
      - "patch"
```

Apply the same policy to the Cargo group. Major upgrades should remain individual PRs so each breaking migration has a reviewable diff and attributable CI result.

## Relationship to usage-dashboard PR #6

These failures are independent of PR #6:

- PR #4 and PR #5 were created and tested against base `57305f5b` before PR #6.
- PR #6 does not modify `package.json`, `pnpm-lock.yaml`, `tsconfig.json`, `Cargo.toml`, or `Cargo.lock`.
- PR #6's latest Backend Checks, Frontend Checks, and label check are green.
- Merging PR #6 does not automatically repair Dependabot PRs. Rebasing #4/#5 onto a newer `main` will trigger a new CI run and may reveal compatibility issues with newer application code.

PR #6 green evidence:

- [Backend Checks](https://github.com/Xr810/LLM-Usage-Bar/actions/runs/29163135165/job/86571534528)
- [Frontend Checks](https://github.com/Xr810/LLM-Usage-Bar/actions/runs/29163135165/job/86571534510)

## Non-blocking Dependabot label comments

The single comment shown on several Dependabot PRs reports that some labels named in `.github/dependabot.yml` did not exist when Dependabot created the PRs. The repository's label workflow later attached available labels. Those comments are configuration warnings and are not the red CI failures described above.

## Suggested repair order for the next task

1. Work on PR #4 and PR #5 independently; do not combine frontend and Rust dependency migrations.
2. First change Dependabot grouping so future major upgrades are not bundled with minor/patch updates.
3. Prefer closing and regenerating the two giant PRs as smaller updates unless there is an explicit requirement to adopt every major version now.
4. If repairing the existing PRs, start from each PR's own branch and reproduce its current failure before editing.
5. Use test-driven, one-cause-at-a-time changes and run the entire ecosystem gate after each error group is green.
6. Record any newly exposed failures here or in a successor task-state document.

## Ready-to-use prompt for a new Codex task

```text
请读取 docs/task-state/2026-07-12-dependabot-ci-failures-handoff.md，分别修复 Dependabot PR #4 和 #5。先更新 Dependabot 分组策略，避免 major 更新继续与 minor/patch 混组；然后对每个 PR 独立复现、修复、运行完整 CI，并持续检查到该 PR 全绿。不要修改或提交我在 main 工作树中现有的 package.json 未提交改动。
```

## Local workspace warning

At the time of this handoff, local `main` contains a pre-existing uncommitted `package.json` change that adds a `packageManager` field. It belongs to the user and must not be overwritten, staged, or included in dependency-repair commits unless the user explicitly requests it.
