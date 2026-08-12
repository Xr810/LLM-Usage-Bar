# Worktree Cargo Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every local Cargo/Tauri build into a Git-common, `Cargo.lock`-hashed target bucket and provide a dry-run-first pruner that never deletes active or referenced caches.

**Architecture:** Cross-platform Node scripts resolve the Git common directory, hash `src-tauri/Cargo.lock`, set `CARGO_TARGET_DIR`, and maintain process leases. A separate planner computes referenced buckets from real Git worktrees; deletion is opt-in and refuses live leases. Package scripts and `AGENTS.md` make the wrapper the default local entry point.

**Tech Stack:** Node.js ESM, `node:test`, Git CLI, pnpm, Cargo/Tauri.

## Global Constraints

- Work in `/Users/max/LLM Usage Bar/.worktrees/full-identity-sync-cache-migration`, never in `main`.
- Do not recreate per-worktree `src-tauri/target` directories during this plan.
- Cache root is `<git-common-parent>/.cache/cargo-targets/<sha256(Cargo.lock)>`.
- Different lockfiles never share a target bucket.
- Dry-run is the default; `--apply` is required for deletion.
- A referenced bucket or bucket with a live process lease is never deleted.
- Preserve the managed Kimi delegation block verbatim in repository `AGENTS.md`.
- CI may continue to use direct Cargo in its ephemeral checkout; local agents use the wrapper.

---

### Task 1: Resolve a stable lock-hashed target directory

**Files:**
- Create: `scripts/cargo-cache-lib.mjs`
- Create: `scripts/cargo-cache-lib.test.mjs`

**Interfaces:**
- Produces: `sha256File(path) -> string`, `gitCommonRoot(cwd) -> string`, `resolveCargoTarget(cwd) -> { commonRoot, lockHash, targetDir, lockfile }`.
- Consumes: real Git repositories and `src-tauri/Cargo.lock`; no mocked Git behavior.

- [ ] **Step 1: Write failing real-repository tests**

```js
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readdir, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { resolveCargoTarget } from "./cargo-cache-lib.mjs";

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
}

async function makeRepo(lockText) {
  const root = await mkdtemp(path.join(tmpdir(), "llm-cache-"));
  git(root, "init");
  git(root, "config", "user.email", "cache@test.invalid");
  git(root, "config", "user.name", "Cache Test");
  await mkdir(path.join(root, "src-tauri"));
  await writeFile(path.join(root, "src-tauri/Cargo.lock"), lockText);
  git(root, "add", ".");
  git(root, "commit", "-m", "fixture");
  return root;
}

test("main and linked worktree with the same lockfile share one bucket", async () => {
  const root = await makeRepo("version = 4\n");
  const linked = `${root}-linked`;
  git(root, "worktree", "add", "-b", "linked", linked);
  assert.equal(resolveCargoTarget(root).targetDir, resolveCargoTarget(linked).targetDir);
});

test("different lockfile contents select different buckets", async () => {
  const first = await makeRepo("version = 4\n[[package]]\nname = 'one'\n");
  const second = await makeRepo("version = 4\n[[package]]\nname = 'two'\n");
  const a = resolveCargoTarget(first);
  const b = resolveCargoTarget(second);
  assert.notEqual(a.lockHash, b.lockHash);
  assert.notEqual(a.targetDir, b.targetDir);
  assert.match(a.targetDir, /[\\/]\.cache[\\/]cargo-targets[\\/][a-f0-9]{64}$/);
  assert.match(b.targetDir, /[\\/]\.cache[\\/]cargo-targets[\\/][a-f0-9]{64}$/);
});
```

- [ ] **Step 2: Verify RED**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: FAIL with `ERR_MODULE_NOT_FOUND` for `cargo-cache-lib.mjs`.

- [ ] **Step 3: Implement the resolver**

```js
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";

export function sha256File(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function gitPath(cwd, argument) {
  return execFileSync("git", ["rev-parse", "--path-format=absolute", argument], {
    cwd,
    encoding: "utf8",
  }).trim();
}

export function gitCommonRoot(cwd) {
  const common = gitPath(cwd, "--git-common-dir");
  return path.dirname(common);
}

export function resolveCargoTarget(cwd = process.cwd()) {
  const commonRoot = gitCommonRoot(cwd);
  const worktreeRoot = gitPath(cwd, "--show-toplevel");
  const lockfile = path.join(worktreeRoot, "src-tauri", "Cargo.lock");
  const lockHash = sha256File(lockfile);
  return {
    commonRoot,
    lockfile,
    lockHash,
    targetDir: path.join(commonRoot, ".cache", "cargo-targets", lockHash),
  };
}
```

Complete the second test with a helper that creates real repositories; do not mock `execFileSync`.

- [ ] **Step 4: Verify GREEN**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: 2 tests pass, 0 fail.

- [ ] **Step 5: Commit**

```bash
git add scripts/cargo-cache-lib.mjs scripts/cargo-cache-lib.test.mjs
git commit -m "build: resolve shared Cargo target buckets"
```

### Task 2: Wrap Cargo/Tauri commands with active leases

**Files:**
- Modify: `scripts/cargo-cache-lib.mjs`
- Modify: `scripts/cargo-cache-lib.test.mjs`
- Create: `scripts/with-cargo-target.mjs`

**Interfaces:**
- Produces: `writeLeaseAtomic(targetDir, metadata)`, `probeBuildProcessTree(lease)`, `removeLease(path)`, and CLI `node scripts/with-cargo-target.mjs <command> [args...]`.
- Produces `resolveBuildCommand`: Cargo resolves to a real `cargo`/`cargo.exe`; Tauri resolves its package bin and launches it with `process.execPath`. Batch/shell wrappers are rejected.
- Lease JSON contains wrapper/direct-child identities plus a process-group/tree safety state, `startedAt`, `cwd`, and `command`; it lives under `<bucket>/.active/`.

- [ ] **Step 1: Add failing lease and child-process tests**

```js
test("wrapper exposes target directory and propagates child exit code", async () => {
  const result = spawnSync(process.execPath, [
    "scripts/with-cargo-target.mjs",
    process.execPath,
    "-e",
    "if (!process.env.CARGO_TARGET_DIR) process.exit(9); process.exit(7)",
  ]);
  assert.equal(result.status, 7);
});

test("lease is removed after a successful child exit", async () => {
  const root = await makeRepo("version = 4\n");
  const wrapper = path.resolve("scripts/with-cargo-target.mjs");
  const result = spawnSync(process.execPath, [wrapper, process.execPath, "-e", "process.exit(0)"], {
    cwd: root,
  });
  assert.equal(result.status, 0);
  const { targetDir } = resolveCargoTarget(root);
  const active = await readdir(path.join(targetDir, ".active"));
  assert.deepEqual(active, []);
});
```

- [ ] **Step 2: Verify RED**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: FAIL because the wrapper and lease functions do not exist.

- [ ] **Step 3: Implement leases and wrapper**

```js
#!/usr/bin/env node
import { mkdirSync } from "node:fs";
import { spawn } from "node:child_process";
import path from "node:path";
import {
  currentBootIdentity,
  pendingLease,
  finalizeLeaseAfterTreeExit,
  removeLease,
  resolveBuildCommand,
  resolveCargoTarget,
  runningLease,
  writeLeaseAtomic,
} from "./cargo-cache-lib.mjs";

const [command, ...args] = process.argv.slice(2);
if (!command) {
  console.error("usage: with-cargo-target <command> [args...]");
  process.exit(64);
}

const resolved = resolveCargoTarget();
const activeDir = path.join(resolved.targetDir, ".active");
mkdirSync(activeDir, { recursive: true });
const lease = path.join(activeDir, `${process.pid}-${Date.now()}.json`);
writeLeaseAtomic(lease, pendingLease(process.pid, process.cwd(), [command, ...args]));

const invocation = resolveBuildCommand(command, args);
const child = spawn(invocation.executable, invocation.args, {
  stdio: "inherit",
  env: { ...process.env, CARGO_TARGET_DIR: resolved.targetDir },
  shell: false,
  detached: process.platform !== "win32",
});
writeLeaseAtomic(lease, runningLease({
  wrapperPid: process.pid,
  childPid: child.pid,
  processGroupId: process.platform === "win32" ? null : child.pid,
  bootIdentity: currentBootIdentity(),
  cwd: process.cwd(),
  command: [command, ...args],
}));

const signalHandlers = new Map();
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  const handler = () => child.kill(signal);
  signalHandlers.set(signal, handler);
  process.on(signal, handler);
}

child.on("error", (error) => {
  removeLease(lease);
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  finalizeLeaseAfterTreeExit(lease);
  for (const [name, handler] of signalHandlers) process.off(name, handler);
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
```

Extract command resolution, atomic lease creation/replacement/removal, boot identity, and process-tree probing into the library so tests use the real functions. Never use `shell: true`: resolve Cargo to the real executable and Tauri to the `@tauri-apps/cli` JavaScript bin launched by `process.execPath`. On macOS/Linux place the child in a dedicated process group and keep the lease while any group member exists. On Windows enumerate the direct child's descendant tree; if enumeration is unavailable or an unclean lease belongs to the current boot, fail closed and keep it until a later boot proves the whole old process tree ended. A direct child exit calls `finalizeLeaseAfterTreeExit`: remove only when the entire group/tree is definitively empty, otherwise atomically mark the lease orphaned. This protects Cargo/rustc descendants after wrapper or intermediate process exit. Lease cleanup must be idempotent. The wrapper must remove its own signal handlers before re-raising a child termination signal; otherwise the wrapper can catch its own signal forever. Add bounded tests for signal propagation, wrapper SIGKILL, and an intermediate child that exits while a grandchild continues writing.

- [ ] **Step 4: Verify GREEN**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: all resolver, lease, and exit propagation tests pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/cargo-cache-lib.mjs scripts/cargo-cache-lib.test.mjs scripts/with-cargo-target.mjs
git commit -m "build: wrap Cargo commands with shared cache leases"
```

### Task 3: Add a dry-run-first cache pruner

**Files:**
- Modify: `scripts/cargo-cache-lib.mjs`
- Modify: `scripts/cargo-cache-lib.test.mjs`
- Create: `scripts/cargo-cache.mjs`

**Interfaces:**
- Produces: `planPrune({ referencedHashes, buckets, probeBuildProcessTree }) -> { keep, remove, unsafeReasons }` and an apply-time revalidation pass.
- CLI: `node scripts/cargo-cache.mjs status` and `node scripts/cargo-cache.mjs prune [--apply]`.

- [ ] **Step 1: Write failing planner tests**

```js
test("prune keeps referenced and active buckets", () => {
  const plan = planPrune({
    referencedHashes: new Set(["current"]),
    buckets: [
      { hash: "current", liveLeases: [] },
      { hash: "active", liveLeases: [{ state: "running", child: { pid: process.pid, startIdentity: "self" } }] },
      { hash: "old", liveLeases: [] },
    ],
    probeBuildProcessTree: (lease) => lease.child.pid === process.pid
      ? { state: "active", members: [process.pid] }
      : { state: "absent" },
  });
  assert.deepEqual(plan.remove.map((item) => item.hash), ["old"]);
  assert.deepEqual(plan.keep.map((item) => item.hash).sort(), ["active", "current"]);
});

test("dry run leaves an unreferenced bucket on disk", async () => {
  const root = await makeRepo("version = 4\n");
  const { targetDir } = resolveCargoTarget(root);
  const stale = path.join(path.dirname(targetDir), "f".repeat(64));
  await mkdir(stale, { recursive: true });
  const cli = path.resolve("scripts/cargo-cache.mjs");
  const result = spawnSync(process.execPath, [cli, "prune"], { cwd: root, encoding: "utf8" });
  assert.equal(result.status, 0);
  assert.equal((await stat(stale)).isDirectory(), true);
  assert.match(result.stdout, /dry run/i);
});
```

- [ ] **Step 2: Verify RED**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: FAIL because `planPrune` and the CLI do not exist.

- [ ] **Step 3: Implement planner and CLI**

The implementation must:

```js
export function planPrune({ referencedHashes, buckets, probeBuildProcessTree }) {
  const keep = [];
  const remove = [];
  const unsafeReasons = [];
  for (const bucket of buckets) {
    const leaseState = classifyBucketLeases(bucket, probeBuildProcessTree);
    if (leaseState.unsafeReason) unsafeReasons.push(leaseState.unsafeReason);
    (referencedHashes.has(bucket.hash) || leaseState.keep ? keep : remove).push(bucket);
  }
  return { keep, remove, unsafeReasons };
}
```

Parse `git worktree list --porcelain`, hash every existing worktree lockfile, parse complete lease JSON, and validate the recorded process group/tree, boot identity, and process start identities. Failure to enumerate a worktree or read/hash any referenced lockfile aborts apply. A malformed/unreadable/pending/orphaned-current-boot lease or indeterminate group/tree check is unsafe and keeps the entire bucket. Write leases by temporary file plus atomic rename so a concurrent reader never accepts half JSON. Remove a stale lease only after the whole recorded group/tree is definitively absent; a boot-identity change may prove all processes from an old Windows lease ended. `prune` prints the plan and every unsafe reason; only `prune --apply` deletes. Immediately before each deletion, re-enumerate worktrees and leases and abort if the bucket became referenced/active or state cannot be verified; then call `rmSync(bucket.path, { recursive: true, force: false })`. Stop on the first deletion error and set a nonzero exit code.

- [ ] **Step 4: Verify GREEN**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: planner, dry-run, apply, active/orphan-grandchild lease, PID-reuse, Windows current/old-boot behavior, half-written/malformed lease, worktree-scan failure, delete-race, and stale lease tests all pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/cargo-cache-lib.mjs scripts/cargo-cache-lib.test.mjs scripts/cargo-cache.mjs
git commit -m "build: add safe Cargo cache pruning"
```

### Task 4: Make the cache policy the project default

**Files:**
- Modify: `package.json`
- Modify: `.gitignore`
- Create: `AGENTS.md`
- Create: `docs/guides/worktree-cargo-cache.md`
- Test: `scripts/cargo-cache-lib.test.mjs`

**Interfaces:**
- Produces pnpm commands: `pnpm tauri`, `pnpm dev`, `pnpm build`, `pnpm rust`, `pnpm cargo:cache`.

- [ ] **Step 1: Add a failing manifest-policy test**

```js
test("project scripts route local Cargo and Tauri through the wrapper", () => {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url)));
  assert.equal(pkg.scripts.tauri, "node scripts/with-cargo-target.mjs tauri");
  assert.equal(pkg.scripts.rust, "node scripts/with-cargo-target.mjs cargo");
  assert.equal(pkg.scripts["cargo:cache"], "node scripts/cargo-cache.mjs");
});
```

- [ ] **Step 2: Verify RED**

Run: `node --test scripts/cargo-cache-lib.test.mjs`

Expected: FAIL because current scripts invoke raw Tauri and have no `rust`/`cargo:cache` entry.

- [ ] **Step 3: Update scripts and policy files**

Set:

```json
{
  "scripts": {
    "dev": "pnpm tauri dev",
    "build": "pnpm tauri build",
    "tauri": "node scripts/with-cargo-target.mjs tauri",
    "rust": "node scripts/with-cargo-target.mjs cargo",
    "cargo:cache": "node scripts/cargo-cache.mjs"
  }
}
```

Append `.cache/cargo-targets/` to `.gitignore`.

Create `AGENTS.md` with the following managed block verbatim:

```markdown
<!-- BEGIN KIMI CODING PLAN DELEGATION (managed) -->
# Kimi Coding Plan 委派规则

- 每次考虑委派前，先调用 `kimi_delegation_status`；总闸关闭或状态不可用时，由 Codex 自己完成。
- 只委派低风险、边界清楚的 Git 任务：测试、lint、简单 bug、机械重构、文档、类型标注和样板代码。
- 禁止委派架构、安全、认证与权限、数据库迁移、依赖升级。
- Codex 必须按顺序读完整个候选补丁，取得 review token 后才能 apply。
- apply 后必须在真实工作区复测；成功则 accept，失败则 rollback。
- 任何 Kimi 失败都立即回到 Codex；同一任务不自动重试 Kimi。
<!-- END KIMI CODING PLAN DELEGATION (managed) -->
```

Then add:

```markdown
## Local Cargo build cache

- Run local Rust commands through `pnpm rust -- <cargo arguments>`.
- Run Tauri through `pnpm tauri -- <arguments>` or `pnpm dev`/`pnpm build`.
- Do not invoke local `cargo build`, `cargo test`, `cargo clippy`, or `tauri` directly.
- Run `pnpm cargo:cache -- status` before removing a worktree.
- `pnpm cargo:cache -- prune` is dry-run; deletion requires `--apply`.
```

Document Windows/macOS/Linux usage, lock-hash isolation, leases, dry-run behavior, and CI exemption in `docs/guides/worktree-cargo-cache.md`.

- [ ] **Step 4: Verify GREEN and no legacy target recreation**

Run:

```bash
node --test scripts/cargo-cache-lib.test.mjs
pnpm cargo:cache -- status
test ! -d src-tauri/target
git diff --check
```

Expected: tests pass; status names the current lock bucket; `src-tauri/target` is absent; diff check passes.

- [ ] **Step 5: Commit**

```bash
git add package.json .gitignore AGENTS.md docs/guides/worktree-cargo-cache.md scripts/cargo-cache-lib.test.mjs
git commit -m "build: enforce worktree-safe Cargo caching"
```

### Task 5: Validate the wrapper on real local commands

**Files:**
- Modify only if verification exposes a defect in Task 1-4 files.

**Interfaces:**
- Consumes the completed cache wrapper; produces a reusable shared bucket for later plans.

- [ ] **Step 1: Install frontend dependencies without building Rust**

Run: `pnpm install --frozen-lockfile`

Expected: exit 0; no `src-tauri/target` directory.

- [ ] **Step 2: Run lightweight Cargo commands through the wrapper**

Run:

```bash
pnpm rust -- metadata --manifest-path src-tauri/Cargo.toml --no-deps --format-version 1
pnpm rust -- fmt --check --manifest-path src-tauri/Cargo.toml
```

Expected: both exit 0; the target directory reported by `pnpm cargo:cache -- status` is under the main checkout `.cache/cargo-targets`; no worktree-local target exists.

- [ ] **Step 3: Run all cache tests and inspect leases**

Run:

```bash
node --test scripts/cargo-cache-lib.test.mjs
pnpm cargo:cache -- status
pnpm cargo:cache -- prune
test ! -d src-tauri/target
git status --short --branch
```

Expected: all tests pass; prune reports dry-run; no live lease remains after commands; only intentional commits differ from the base.

- [ ] **Step 4: Commit verification fixes only if needed**

```bash
git add scripts package.json .gitignore AGENTS.md docs/guides/worktree-cargo-cache.md
git commit -m "fix(build): harden shared Cargo cache tooling"
```

If no fix is needed, do not create an empty commit.
