import test from "node:test";
import assert from "node:assert/strict";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  readdir,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import {
  applyPrunePlan,
  createPruneSnapshot,
  currentBootIdentity,
  finalizeLeaseAfterTreeExit,
  planPrune,
  probeBuildProcessTree,
  removeLease,
  resolveBuildCommand,
  resolveCargoTarget,
  writeLeaseAtomic,
} from "./cargo-cache-lib.mjs";

const wrapperPath = path.resolve("scripts/with-cargo-target.mjs");
const cacheCliPath = path.resolve("scripts/cargo-cache.mjs");

function readBooleanMapping(yaml, mappingName) {
  const lines = yaml.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === `${mappingName}:`);
  assert.notEqual(start, -1, `missing ${mappingName} mapping`);

  const values = new Map();
  for (const line of lines.slice(start + 1)) {
    if (line.trim() === "") continue;
    if (!/^\s/.test(line)) break;
    const entry = line.match(
      /^\s+(?:'([^']+)'|"([^"]+)"|([^:]+)):\s*(true|false)\s*$/,
    );
    assert.ok(entry, `invalid ${mappingName} entry: ${line}`);
    values.set(entry[1] ?? entry[2] ?? entry[3].trim(), entry[4] === "true");
  }
  return values;
}

function workflowPnpmPins(workflow) {
  const actionPins = Array.from(
    workflow.matchAll(/^\s*version:\s*([0-9.]+)\s*$/gm),
    (match) => match[1],
  );
  const corepackPins = Array.from(
    workflow.matchAll(/corepack prepare pnpm@([0-9.]+) --activate/g),
    (match) => match[1],
  );
  return [...actionPins, ...corepackPins];
}

test("project scripts route local Cargo and Tauri through the wrapper", () => {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url)));
  assert.equal(pkg.scripts.tauri, "node scripts/with-cargo-target.mjs tauri");
  assert.equal(pkg.scripts.rust, "node scripts/with-cargo-target.mjs cargo");
  assert.equal(pkg.scripts["cargo:cache"], "node scripts/cargo-cache.mjs");
});

test("package metadata pins the pnpm build-policy runtime", () => {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url)));
  assert.equal(pkg.packageManager, "pnpm@11.11.0");
});

test("pnpm 11 build policy explicitly allows trusted native builds", () => {
  const workspace = readFileSync(
    new URL("../pnpm-workspace.yaml", import.meta.url),
    "utf8",
  );
  const allowBuilds = readBooleanMapping(workspace, "allowBuilds");

  assert.equal(allowBuilds.get("@tailwindcss/oxide"), true);
  assert.equal(allowBuilds.get("esbuild"), true);
  assert.equal(allowBuilds.get("msw"), false);
  assert.doesNotMatch(
    workspace,
    /^\s*(?:onlyBuiltDependencies|ignoredBuiltDependencies):/m,
  );
  assert.doesNotMatch(workspace, /set this to true or false/);
});

test("CI and release workflows pin the pnpm build-policy runtime", () => {
  const ci = readFileSync(
    new URL("../.github/workflows/ci.yml", import.meta.url),
    "utf8",
  );
  const release = readFileSync(
    new URL("../.github/workflows/release.yml", import.meta.url),
    "utf8",
  );

  assert.doesNotMatch(ci, /10\.12\.3/);
  assert.doesNotMatch(release, /10\.12\.3/);
  assert.deepEqual(workflowPnpmPins(ci), ["11.11.0"]);
  assert.deepEqual(workflowPnpmPins(release), ["11.11.0"]);
});

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
}

async function makeRepo(lockText) {
  const root = await mkdtemp(path.join(tmpdir(), "llm-cache-"));
  git(root, "init");
  git(root, "config", "user.email", "cache@test.invalid");
  git(root, "config", "user.name", "Cache Test");
  await mkdir(path.join(root, "src-tauri"));
  await writeFile(path.join(root, "src-tauri", "Cargo.lock"), lockText);
  git(root, "add", ".");
  git(root, "commit", "-m", "fixture");
  return root;
}

async function waitUntil(check, description, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;

  while (Date.now() < deadline) {
    try {
      const result = await check();
      if (result) return result;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
  }

  const detail = lastError ? `: ${lastError.message}` : "";
  throw new Error(`timed out waiting for ${description}${detail}`);
}

function waitForExit(child, timeoutMs = 5_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve({ code: child.exitCode, signal: child.signalCode });
  }

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`process ${child.pid} did not exit within ${timeoutMs}ms`));
    }, timeoutMs);
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      resolve({ code, signal });
    });
  });
}

async function readLeases(root) {
  const { targetDir } = resolveCargoTarget(root);
  const activeDir = path.join(targetDir, ".active");
  let names;
  try {
    names = await readdir(activeDir);
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }

  return Promise.all(
    names
      .filter((name) => name.endsWith(".json"))
      .map(async (name) => {
        const leasePath = path.join(activeDir, name);
        return {
          leasePath,
          metadata: JSON.parse(await readFile(leasePath, "utf8")),
        };
      }),
  );
}

async function waitForLease(root, predicate = () => true) {
  return waitUntil(async () => {
    const leases = await readLeases(root);
    return leases.find(({ metadata }) => predicate(metadata));
  }, "matching Cargo cache lease");
}

async function cleanupRepoProcesses(root) {
  const leases = await readLeases(root);
  for (const { leasePath, metadata } of leases) {
    const groupId = metadata.processTree?.processGroupId;
    const childPid = metadata.child?.pid;
    try {
      if (process.platform !== "win32" && Number.isSafeInteger(groupId)) {
        process.kill(-groupId, "SIGKILL");
      } else if (Number.isSafeInteger(childPid)) {
        process.kill(childPid, "SIGKILL");
      }
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
    removeLease(leasePath);
  }
}

function spawnWrapper(root, command, args, options = {}) {
  return spawn(process.execPath, [wrapperPath, command, ...args], {
    cwd: root,
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  });
}

function knownBootIdentityPattern() {
  if (process.platform === "linux") {
    return /^linux:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;
  }
  if (process.platform === "darwin") return /^darwin:\d+:\d{1,6}$/;
  if (process.platform === "win32") {
    return /^win32:\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,7})?Z$/;
  }
  return null;
}

function previousKnownBootIdentity() {
  if (process.platform === "linux") {
    return "linux:00000000-0000-0000-0000-000000000001";
  }
  if (process.platform === "darwin") return "darwin:1:0";
  if (process.platform === "win32") {
    return "win32:2000-01-01T00:00:00.0000000Z";
  }
  return null;
}

function invalidKnownBootIdentityShape() {
  if (process.platform === "linux") {
    return "linux:00000000-0000-0000-0000-00000000000g";
  }
  if (process.platform === "darwin") return "darwin:9007199254740992:0";
  if (process.platform === "win32") {
    return "win32:2026-02-30T00:00:00.0000000Z";
  }
  return `${process.platform}:unsupported`;
}

function runCacheCli(root, ...args) {
  return spawnSync(process.execPath, [cacheCliPath, ...args], {
    cwd: root,
    encoding: "utf8",
    timeout: 5_000,
  });
}

async function makeCacheBucket(root, hash) {
  const { targetDir } = resolveCargoTarget(root);
  const bucket = path.join(path.dirname(targetDir), hash);
  await mkdir(bucket, { recursive: true });
  return bucket;
}

test("prune keeps referenced and active buckets", () => {
  const plan = planPrune({
    referencedHashes: new Set(["current"]),
    buckets: [
      { hash: "current", liveLeases: [] },
      {
        hash: "active",
        liveLeases: [
          {
            state: "running",
            child: { pid: process.pid, startedAtToken: "self" },
          },
        ],
      },
      { hash: "old", liveLeases: [] },
    ],
    probeBuildProcessTree: (lease) =>
      lease.child.pid === process.pid
        ? { state: "active", members: [process.pid] }
        : { state: "empty" },
  });

  assert.deepEqual(plan.remove.map((item) => item.hash), ["old"]);
  assert.deepEqual(
    plan.keep.map((item) => item.hash).sort(),
    ["active", "current"],
  );
  assert.deepEqual(plan.unsafeReasons, []);
});

test("dry run leaves an unreferenced bucket on disk", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const { targetDir } = resolveCargoTarget(root);
  const stale = path.join(path.dirname(targetDir), "f".repeat(64));
  await mkdir(stale, { recursive: true });

  const result = spawnSync(process.execPath, [cacheCliPath, "prune"], {
    cwd: root,
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal((await stat(stale)).isDirectory(), true);
  assert.match(result.stdout, /dry run/i);
  assert.doesNotMatch(result.stdout, /current lock:|referenced lock:/);
});

test("pending leases fail closed without probing a process tree", () => {
  const hash = "a".repeat(64);
  const plan = planPrune({
    referencedHashes: new Set(),
    buckets: [{ hash, liveLeases: [{ state: "pending" }] }],
    probeBuildProcessTree: () => {
      throw new Error("pending leases must not be probed as removable");
    },
  });

  assert.deepEqual(plan.remove, []);
  assert.deepEqual(plan.keep.map((bucket) => bucket.hash), [hash]);
  assert.match(plan.unsafeReasons.join("\n"), /pending/i);
});

test("PID reuse and indeterminate current-boot leases fail closed", () => {
  const pidReuse = "b".repeat(64);
  const currentBootOrphan = "c".repeat(64);
  const plan = planPrune({
    referencedHashes: new Set(),
    buckets: [
      { hash: pidReuse, liveLeases: [{ state: "running" }] },
      { hash: currentBootOrphan, liveLeases: [{ state: "orphaned" }] },
    ],
    probeBuildProcessTree: (lease) =>
      lease.state === "running"
        ? { state: "unknown", reason: "child-pid-reused" }
        : { state: "unknown", reason: "current-boot-tree-indeterminate" },
  });

  assert.deepEqual(plan.remove, []);
  assert.deepEqual(
    plan.keep.map((bucket) => bucket.hash),
    [pidReuse, currentBootOrphan],
  );
  assert.match(plan.unsafeReasons.join("\n"), /child-pid-reused/);
  assert.match(plan.unsafeReasons.join("\n"), /current-boot-tree-indeterminate/);
});

test("a definitively empty old-boot lease is removable", () => {
  const hash = "d".repeat(64);
  const plan = planPrune({
    referencedHashes: new Set(),
    buckets: [{ hash, liveLeases: [{ state: "orphaned" }] }],
    probeBuildProcessTree: () => ({ state: "empty", reason: "previous-boot" }),
  });

  assert.deepEqual(plan.keep, []);
  assert.deepEqual(plan.remove.map((bucket) => bucket.hash), [hash]);
  assert.deepEqual(plan.unsafeReasons, []);
});

test("a current-boot orphan remains fail-closed even if one probe is empty", () => {
  const hash = "0".repeat(64);
  const plan = planPrune({
    referencedHashes: new Set(),
    buckets: [
      {
        hash,
        liveLeases: [
          {
            state: "orphaned",
            bootIdentity: currentBootIdentity(),
          },
        ],
      },
    ],
    probeBuildProcessTree: () => ({ state: "empty", reason: "group-empty" }),
  });

  assert.deepEqual(plan.remove, []);
  assert.deepEqual(plan.keep.map((bucket) => bucket.hash), [hash]);
  assert.match(plan.unsafeReasons.join("\n"), /current-boot.*orphan/i);
});

test("status is read-only and reports the cache plan", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const stale = await makeCacheBucket(root, "e".repeat(64));

  const result = runCacheCli(root, "status");

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /remove: 1/);
  assert.equal((await stat(stale)).isDirectory(), true);
});

test("status reports the current lock hash before its cache bucket exists", async (t) => {
  const root = await makeRepo("version = 4\n[[package]]\nname = 'identity'\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const { lockHash, targetDir } = resolveCargoTarget(root);
  await assert.rejects(access(targetDir), { code: "ENOENT" });

  const result = runCacheCli(root, "status");

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`current lock: ${lockHash}`));
  assert.match(result.stdout, new RegExp(`referenced lock: ${lockHash}`));
  assert.match(result.stdout, /keep: 0/);
  assert.match(result.stdout, /remove: 0/);
  await assert.rejects(access(targetDir), { code: "ENOENT" });
});

test("cache CLI accepts exactly one leading pnpm separator", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));

  const separated = runCacheCli(root, "--", "status");
  assert.equal(separated.status, 0, separated.stderr);
  assert.match(separated.stdout, /keep: 0/);

  for (const args of [
    ["--", "--", "status"],
    ["--", "status", "--"],
  ]) {
    const invalid = runCacheCli(root, ...args);
    assert.notEqual(invalid.status, 0);
    assert.match(invalid.stderr, /Usage:/);
  }
});

test("apply deletes only stale hash buckets inside the shared cache", async (t) => {
  const root = await makeRepo("version = 4\n");
  const outside = await mkdtemp(path.join(tmpdir(), "llm-cache-outside-"));
  t.after(async () => {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  });
  const stale = await makeCacheBucket(root, "f".repeat(64));
  const { targetDir } = resolveCargoTarget(root);
  const cacheRoot = path.dirname(targetDir);
  const legacyTarget = path.join(root, "target");
  const ignoredBucket = path.join(cacheRoot, "legacy-target");
  const externalMarker = path.join(outside, "must-survive");
  const linkedHash = "1".repeat(64);
  const linkedBucket = path.join(cacheRoot, linkedHash);
  const worktreeMarker = path.join(root, "must-survive");
  await mkdir(legacyTarget);
  await mkdir(ignoredBucket);
  await writeFile(externalMarker, "outside\n");
  await writeFile(worktreeMarker, "worktree\n");
  await symlink(outside, linkedBucket, process.platform === "win32" ? "junction" : "dir");

  const result = runCacheCli(root, "prune", "--apply");

  assert.equal(result.status, 0, result.stderr);
  await assert.rejects(access(stale), { code: "ENOENT" });
  assert.equal((await stat(legacyTarget)).isDirectory(), true);
  assert.equal((await stat(ignoredBucket)).isDirectory(), true);
  assert.equal(await readFile(externalMarker, "utf8"), "outside\n");
  assert.equal(await readFile(worktreeMarker, "utf8"), "worktree\n");
  assert.equal((await stat(linkedBucket)).isDirectory(), true);
});

test("malformed and half-written leases keep their entire buckets", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const malformed = await makeCacheBucket(root, "2".repeat(64));
  const halfWritten = await makeCacheBucket(root, "3".repeat(64));
  await mkdir(path.join(malformed, ".active"));
  await mkdir(path.join(halfWritten, ".active"));
  await writeFile(path.join(malformed, ".active", "lease.json"), "{\"state\":");
  await writeFile(
    path.join(halfWritten, ".active", ".lease.json.123.pending.tmp"),
    "{\"state\":\"pending\"",
  );

  const result = runCacheCli(root, "prune", "--apply");

  assert.equal(result.status, 0, result.stderr);
  assert.equal((await stat(malformed)).isDirectory(), true);
  assert.equal((await stat(halfWritten)).isDirectory(), true);
  assert.match(result.stdout, /unsafe/i);
  assert.match(result.stdout, /malformed|json|temporary|unexpected/i);
});

test("a current-boot orphan with an unknown tree keeps its bucket", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const bucket = await makeCacheBucket(root, "4".repeat(64));
  const leasePath = path.join(bucket, ".active", "orphan.json");
  writeLeaseAtomic(leasePath, {
    version: 1,
    state: "orphaned",
    bootIdentity: currentBootIdentity(),
    processTree: { kind: "unknown" },
  });

  const result = runCacheCli(root, "prune", "--apply");

  assert.equal(result.status, 0, result.stderr);
  assert.equal((await stat(bucket)).isDirectory(), true);
  assert.match(result.stdout, /unsafe/i);
});

test("a valid previous-boot orphan can be deleted", async (t) => {
  const previousBoot = previousKnownBootIdentity();
  if (!previousBoot || previousBoot === currentBootIdentity()) return;
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const bucket = await makeCacheBucket(root, "5".repeat(64));
  writeLeaseAtomic(path.join(bucket, ".active", "old-boot.json"), {
    version: 1,
    state: "orphaned",
    bootIdentity: previousBoot,
    processTree: { kind: "unknown" },
  });

  const result = runCacheCli(root, "prune", "--apply");

  assert.equal(result.status, 0, result.stderr);
  await assert.rejects(access(bucket), { code: "ENOENT" });
});

test(
  "an orphaned build group with a live grandchild keeps its bucket",
  { skip: process.platform === "win32" },
  async (t) => {
    const root = await makeRepo("version = 4\n");
    const readyPath = path.join(root, "orphan-grandchild-ready");
    const grandchildScript = [
      "const fs = require('node:fs');",
      `fs.writeFileSync(${JSON.stringify(readyPath)}, 'ready');`,
      "setInterval(() => {}, 100);",
    ].join(" ");
    const parentScript = [
      "const { spawn } = require('node:child_process');",
      `const child = spawn(process.execPath, ['-e', ${JSON.stringify(grandchildScript)}], { stdio: 'ignore' });`,
      "child.unref();",
      "process.exit(0);",
    ].join(" ");
    const parent = spawn(process.execPath, ["-e", parentScript], {
      cwd: root,
      detached: true,
      stdio: "ignore",
    });
    const processGroupId = parent.pid;
    t.after(async () => {
      try {
        process.kill(-processGroupId, "SIGKILL");
      } catch (error) {
        if (error.code !== "ESRCH") throw error;
      }
      await waitUntil(() => {
        try {
          process.kill(-processGroupId, 0);
          return false;
        } catch (error) {
          if (error.code === "ESRCH") return true;
          throw error;
        }
      }, "orphaned test process group cleanup");
      await rm(root, { recursive: true, force: true });
    });
    await waitForExit(parent);
    await waitUntil(async () => {
      try {
        return (await readFile(readyPath, "utf8")) === "ready";
      } catch {
        return false;
      }
    }, "orphaned grandchild readiness");
    const bucket = await makeCacheBucket(root, "6".repeat(64));
    writeLeaseAtomic(path.join(bucket, ".active", "orphan.json"), {
      version: 1,
      state: "orphaned",
      bootIdentity: currentBootIdentity(),
      child: { pid: processGroupId, startedAtToken: "exited-root" },
      processTree: {
        kind: "unix-process-group",
        processGroupId,
        safetyState: "active",
      },
    });

    const result = runCacheCli(root, "prune", "--apply");

    assert.equal(result.status, 0, result.stderr);
    assert.equal((await stat(bucket)).isDirectory(), true);
  },
);

test("apply aborts before deleting when any worktree lockfile cannot be hashed", async (t) => {
  const root = await makeRepo("version = 4\n");
  const linked = `${root}-broken-linked`;
  git(root, "worktree", "add", "-b", "broken-linked", linked);
  t.after(async () => {
    await rm(linked, { recursive: true, force: true });
    await rm(root, { recursive: true, force: true });
  });
  await rm(path.join(linked, "src-tauri", "Cargo.lock"));
  const stale = await makeCacheBucket(root, "7".repeat(64));

  const result = runCacheCli(root, "prune", "--apply");

  assert.notEqual(result.status, 0);
  assert.equal((await stat(stale)).isDirectory(), true);
  assert.match(result.stderr, /Cargo\.lock|ENOENT|no such file/i);
});

test("apply aborts before deleting when git cannot enumerate worktrees", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const stale = await makeCacheBucket(root, "c".repeat(64));
  const gitDirectory = path.join(root, ".git");
  const hiddenGitDirectory = path.join(root, ".git-hidden-for-cache-test");
  await rename(gitDirectory, hiddenGitDirectory);

  try {
    const result = runCacheCli(root, "prune", "--apply");

    assert.notEqual(result.status, 0);
    assert.equal((await stat(stale)).isDirectory(), true);
    assert.match(result.stderr, /git|repository|worktree/i);
  } finally {
    await rename(hiddenGitDirectory, gitDirectory);
  }
});

test("apply rejects a non-hash planned path before deleting anything", async (t) => {
  const root = await makeRepo("version = 4\n");
  const outside = await mkdtemp(path.join(tmpdir(), "llm-cache-boundary-"));
  const marker = path.join(outside, "must-survive");
  t.after(async () => {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  });
  await writeFile(marker, "outside\n");

  const result = applyPrunePlan({
    cwd: root,
    plannedBuckets: [{ hash: "../outside", path: outside }],
    probeBuildProcessTree,
    removeBucket: () => {
      throw new Error("invalid planned paths must never reach deletion");
    },
  });

  assert.equal(result.ok, false);
  assert.deepEqual(result.deleted, []);
  assert.match(result.error, /non-hash|preflight/i);
  assert.equal(await readFile(marker, "utf8"), "outside\n");
});

test("apply revalidates a bucket that gains a lease after planning", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const bucket = await makeCacheBucket(root, "8".repeat(64));
  const snapshot = createPruneSnapshot(root);
  const plan = planPrune({
    ...snapshot,
    probeBuildProcessTree,
  });
  assert.deepEqual(plan.remove.map((item) => item.hash), ["8".repeat(64)]);
  await mkdir(path.join(bucket, ".active"));
  await writeFile(path.join(bucket, ".active", "raced.json"), "{\"state\":");

  const result = applyPrunePlan({
    cwd: root,
    plannedBuckets: plan.remove,
    probeBuildProcessTree,
  });

  assert.equal(result.ok, false);
  assert.deepEqual(result.deleted, []);
  assert.equal((await stat(bucket)).isDirectory(), true);
  assert.match(result.error, /revalidation|unsafe|lease/i);
});

test("apply revalidates every bucket after an earlier deletion", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const firstHash = "8".repeat(64);
  const secondHash = "9".repeat(64);
  const first = await makeCacheBucket(root, firstHash);
  const second = await makeCacheBucket(root, secondHash);
  const snapshot = createPruneSnapshot(root);
  const plan = planPrune({ ...snapshot, probeBuildProcessTree });

  const result = applyPrunePlan({
    cwd: root,
    plannedBuckets: plan.remove,
    probeBuildProcessTree,
    removeBucket: (bucket) => {
      rmSync(bucket.path, { recursive: true, force: false });
      if (bucket.hash === firstHash) {
        writeLeaseAtomic(path.join(second, ".active", "raced.json"), {
          version: 1,
          state: "pending",
        });
      }
    },
  });

  assert.equal(result.ok, false);
  assert.deepEqual(result.deleted, [firstHash]);
  await assert.rejects(access(first), { code: "ENOENT" });
  assert.equal((await stat(second)).isDirectory(), true);
  assert.match(result.error, /revalidation|pending|unsafe/i);
});

test("apply stops on the first deletion error and reports prior deletions", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(() => rm(root, { recursive: true, force: true }));
  const firstHash = "9".repeat(64);
  const secondHash = "a".repeat(64);
  const thirdHash = "b".repeat(64);
  await makeCacheBucket(root, firstHash);
  await makeCacheBucket(root, secondHash);
  await makeCacheBucket(root, thirdHash);
  const snapshot = createPruneSnapshot(root);
  const plan = planPrune({ ...snapshot, probeBuildProcessTree });

  const result = applyPrunePlan({
    cwd: root,
    plannedBuckets: plan.remove,
    probeBuildProcessTree,
    removeBucket: (bucket) => {
      if (bucket.hash === secondHash) throw new Error("synthetic delete failure");
      rmSync(bucket.path, { recursive: true, force: false });
    },
  });

  assert.equal(result.ok, false);
  assert.deepEqual(result.deleted, [firstHash]);
  assert.equal((await stat(path.join(path.dirname(resolveCargoTarget(root).targetDir), secondHash))).isDirectory(), true);
  assert.equal((await stat(path.join(path.dirname(resolveCargoTarget(root).targetDir), thirdHash))).isDirectory(), true);
  assert.match(result.error, /synthetic delete failure/);
});

test("main and linked worktree with the same lockfile share one bucket", async () => {
  const root = await makeRepo("version = 4\n");
  const linked = `${root}-linked`;
  git(root, "worktree", "add", "-b", "linked", linked);

  assert.equal(
    resolveCargoTarget(root).targetDir,
    resolveCargoTarget(linked).targetDir,
  );
});

test("different lockfile contents select different buckets", async () => {
  const first = await makeRepo("version = 4\n[[package]]\nname = 'one'\n");
  const second = await makeRepo("version = 4\n[[package]]\nname = 'two'\n");
  const a = resolveCargoTarget(first);
  const b = resolveCargoTarget(second);

  assert.notEqual(a.lockHash, b.lockHash);
  assert.notEqual(a.targetDir, b.targetDir);
  assert.match(
    a.targetDir,
    /[\\/]\.cache[\\/]cargo-targets[\\/][a-f0-9]{64}$/,
  );
  assert.match(
    b.targetDir,
    /[\\/]\.cache[\\/]cargo-targets[\\/][a-f0-9]{64}$/,
  );
});

test("build commands resolve to executables without shell wrappers", async (t) => {
  const fixture = await mkdtemp(path.join(tmpdir(), "llm-cache-command-"));
  t.after(() => rm(fixture, { recursive: true, force: true }));

  const cargo = resolveBuildCommand("cargo", ["--version"], fixture);
  assert.match(path.basename(cargo.executable).toLowerCase(), /^cargo(?:\.exe)?$/);
  assert.deepEqual(cargo.args, ["--version"]);

  const packageDir = path.join(
    fixture,
    "node_modules",
    "@tauri-apps",
    "cli",
  );
  await mkdir(packageDir, { recursive: true });
  await writeFile(path.join(fixture, "package.json"), "{\"private\":true}\n");
  await writeFile(
    path.join(packageDir, "package.json"),
    JSON.stringify({
      name: "@tauri-apps/cli",
      version: "0.0.0-test",
      bin: { tauri: "tauri.js" },
    }),
  );
  await writeFile(path.join(packageDir, "tauri.js"), "process.exit(0);\n");

  const tauri = resolveBuildCommand("tauri", ["dev"], fixture);
  assert.equal(tauri.executable, process.execPath);
  assert.equal(tauri.args[0], await realpath(path.join(packageDir, "tauri.js")));
  assert.deepEqual(tauri.args.slice(1), ["dev"]);

  const batch = path.join(fixture, "cargo.cmd");
  await writeFile(batch, "@echo off\r\n");
  await chmod(batch, 0o755);
  assert.throws(
    () => resolveBuildCommand(batch, [], fixture),
    /shell wrapper/i,
  );
});

test("Tauri package-bin symlinks cannot escape the real package root", async (t) => {
  const fixture = await mkdtemp(path.join(tmpdir(), "llm-cache-tauri-link-"));
  const packageDir = path.join(
    fixture,
    "node_modules",
    "@tauri-apps",
    "cli",
  );
  const outsideDir = path.join(fixture, "outside-package");
  const outsideBin = path.join(outsideDir, "tauri.js");
  t.after(() => rm(fixture, { recursive: true, force: true }));

  await mkdir(packageDir, { recursive: true });
  await mkdir(outsideDir);
  await writeFile(path.join(fixture, "package.json"), "{\"private\":true}\n");
  await writeFile(
    path.join(packageDir, "package.json"),
    JSON.stringify({
      name: "@tauri-apps/cli",
      version: "0.0.0-test",
      bin: { tauri: "escaped/tauri.js" },
    }),
  );
  await writeFile(outsideBin, "process.exit(0);\n");
  await symlink(
    outsideDir,
    path.join(packageDir, "escaped"),
    process.platform === "win32" ? "junction" : "dir",
  );

  assert.throws(
    () => resolveBuildCommand("tauri", ["dev"], fixture),
    /package root|regular file|escapes/i,
  );
});

test("Tauri package bin must resolve to a regular file", async (t) => {
  const fixture = await mkdtemp(path.join(tmpdir(), "llm-cache-tauri-file-"));
  const packageDir = path.join(
    fixture,
    "node_modules",
    "@tauri-apps",
    "cli",
  );
  t.after(() => rm(fixture, { recursive: true, force: true }));

  await mkdir(path.join(packageDir, "tauri.js"), { recursive: true });
  await writeFile(path.join(fixture, "package.json"), "{\"private\":true}\n");
  await writeFile(
    path.join(packageDir, "package.json"),
    JSON.stringify({
      name: "@tauri-apps/cli",
      version: "0.0.0-test",
      bin: { tauri: "tauri.js" },
    }),
  );

  assert.throws(
    () => resolveBuildCommand("tauri", ["dev"], fixture),
    /regular file/i,
  );
});

test("lease replacement is atomic and removal is idempotent", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "llm-cache-lease-"));
  const leasePath = path.join(root, ".active", "lease.json");
  t.after(() => rm(root, { recursive: true, force: true }));

  writeLeaseAtomic(leasePath, { state: "pending", revision: 1 });
  writeLeaseAtomic(leasePath, { state: "running", revision: 2 });

  assert.deepEqual(JSON.parse(await readFile(leasePath, "utf8")), {
    state: "running",
    revision: 2,
  });
  assert.deepEqual(await readdir(path.dirname(leasePath)), ["lease.json"]);

  removeLease(leasePath);
  removeLease(leasePath);
  await assert.rejects(access(leasePath), { code: "ENOENT" });
});

test("same-boot ambiguous leases fail closed", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "llm-cache-unknown-"));
  const leasePath = path.join(root, "lease.json");
  t.after(() => rm(root, { recursive: true, force: true }));
  writeLeaseAtomic(leasePath, {
    version: 1,
    state: "running",
    bootIdentity: currentBootIdentity(),
    processTree: { kind: "unknown" },
  });

  assert.equal(probeBuildProcessTree(JSON.parse(await readFile(leasePath))).state, "unknown");
  assert.equal(finalizeLeaseAfterTreeExit(leasePath), false);
  assert.equal(JSON.parse(await readFile(leasePath)).state, "orphaned");
});

test("an unknown boot identity never proves that a lease is from an old boot", () => {
  const probe = probeBuildProcessTree({
    version: 1,
    state: "orphaned",
    bootIdentity: `${process.platform}:unknown`,
    processTree: { kind: "unknown" },
  });

  assert.equal(probe.state, "unknown");
});

test("a malformed or cross-platform boot identity fails closed", () => {
  for (const bootIdentity of [
    "",
    "malformed",
    `${process.platform}:`,
    `${process.platform}:garbage`,
    invalidKnownBootIdentityShape(),
    "another-platform:known-boot",
  ]) {
    const probe = probeBuildProcessTree({
      version: 1,
      state: "orphaned",
      bootIdentity,
      processTree: { kind: "unknown" },
    });

    assert.equal(probe.state, "unknown");
  }
});

test("current boot identities are canonical or explicitly unknown", () => {
  const identity = currentBootIdentity();
  if (identity === `${process.platform}:unknown`) return;

  const pattern = knownBootIdentityPattern();
  assert.ok(pattern, `unsupported known boot identity: ${identity}`);
  assert.match(identity, pattern);
});

test("only a previous valid boot identity proves an old process tree is empty", () => {
  const bootIdentity = previousKnownBootIdentity();
  if (!bootIdentity || bootIdentity === currentBootIdentity()) return;

  assert.deepEqual(
    probeBuildProcessTree({
      version: 1,
      state: "orphaned",
      bootIdentity,
      processTree: { kind: "unknown" },
    }),
    { state: "empty", reason: "previous-boot" },
  );
});

test("Windows snapshot fails closed when exited ancestors hide a deep descendant", async () => {
  const module = await import("./cargo-cache-lib.mjs");
  assert.equal(typeof module.probeWindowsProcessTree, "function");

  const probe = module.probeWindowsProcessTree(
    {
      version: 1,
      state: "orphaned",
      child: { pid: 100, startedAtToken: "root-start" },
      processTree: { kind: "windows-descendants" },
    },
    [
      {
        pid: 300,
        parentPid: 200,
        startedAtToken: "deep-grandchild-start",
      },
    ],
  );

  assert.deepEqual(probe, {
    state: "unknown",
    reason: "windows-root-missing",
  });
});

test("wrapper exposes target directory and propagates child exit code", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(async () => {
    await cleanupRepoProcesses(root);
    await rm(root, { recursive: true, force: true });
  });

  const result = spawnSync(
    process.execPath,
    [
      wrapperPath,
      process.execPath,
      "-e",
      "if (!process.env.CARGO_TARGET_DIR) process.exit(9); process.exit(7)",
    ],
    { cwd: root },
  );
  assert.equal(result.status, 7, result.stderr?.toString());
});

test("wrapper strips exactly one pnpm separator and preserves later separators", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(async () => {
    await cleanupRepoProcesses(root);
    await rm(root, { recursive: true, force: true });
  });

  const separated = spawnSync(
    process.execPath,
    [
      wrapperPath,
      process.execPath,
      "--",
      "-e",
      "process.stdout.write('separated')",
    ],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(separated.status, 0, separated.stderr);
  assert.equal(separated.stdout, "separated");

  const doubled = spawnSync(
    process.execPath,
    [
      wrapperPath,
      process.execPath,
      "--",
      "--",
      "-e",
      "process.exit(0)",
    ],
    { cwd: root, encoding: "utf8" },
  );
  assert.notEqual(doubled.status, 0);

  const fixture = path.join(root, "argv-fixture.mjs");
  await writeFile(
    fixture,
    "process.stdout.write(JSON.stringify(process.argv.slice(2)));\n",
  );
  const preserved = spawnSync(
    process.execPath,
    [wrapperPath, process.execPath, fixture, "--", "payload"],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(preserved.status, 0, preserved.stderr);
  assert.deepEqual(JSON.parse(preserved.stdout), ["--", "payload"]);
});

test("successful child exit removes a lease only when tree emptiness is provable", async (t) => {
  const root = await makeRepo("version = 4\n");
  t.after(async () => {
    await cleanupRepoProcesses(root);
    await rm(root, { recursive: true, force: true });
  });

  const result = spawnSync(
    process.execPath,
    [wrapperPath, process.execPath, "-e", "process.exit(0)"],
    { cwd: root },
  );
  assert.equal(result.status, 0, result.stderr?.toString());
  const { targetDir } = resolveCargoTarget(root);
  const activeDir = path.join(targetDir, ".active");
  const active = await readdir(activeDir);
  if (process.platform === "win32") {
    assert.equal(active.length, 1);
    const lease = JSON.parse(
      await readFile(path.join(activeDir, active[0]), "utf8"),
    );
    assert.equal(lease.state, "orphaned");
    assert.equal(lease.lastProbe?.state, "unknown");
  } else {
    assert.deepEqual(active, []);
  }
});

test(
  "SIGTERM reaches the build group before the wrapper re-raises it",
  { skip: process.platform === "win32" },
  async (t) => {
    const root = await makeRepo("version = 4\n");
    const wrapper = spawnWrapper(root, process.execPath, [
      "-e",
      "setInterval(() => {}, 1_000)",
    ]);
    t.after(async () => {
      if (wrapper.exitCode === null && wrapper.signalCode === null) {
        wrapper.kill("SIGKILL");
      }
      await cleanupRepoProcesses(root);
      await rm(root, { recursive: true, force: true });
    });

    await waitForLease(root, (lease) => lease.state === "running");
    const exit = waitForExit(wrapper);
    wrapper.kill("SIGTERM");
    assert.deepEqual(await exit, { code: null, signal: "SIGTERM" });

    await waitUntil(async () => {
      const leases = await readLeases(root);
      for (const { leasePath, metadata } of leases) {
        if (probeBuildProcessTree(metadata).state !== "empty") return false;
        finalizeLeaseAfterTreeExit(leasePath);
      }
      return (await readLeases(root)).length === 0;
    }, "terminated build group to become empty");
  },
);

test(
  "a signal raised inside spawn is queued and forwarded once the child exists",
  { skip: process.platform === "win32" },
  async (t) => {
    const root = await makeRepo("version = 4\n");
    const preloadPath = path.join(root, "signal-during-spawn.mjs");
    await writeFile(
      preloadPath,
      [
        "import childProcess from 'node:child_process';",
        "import { syncBuiltinESMExports } from 'node:module';",
        "const realSpawn = childProcess.spawn;",
        "childProcess.spawn = function (...args) {",
        "  const child = realSpawn.apply(this, args);",
        "  process.emit('SIGTERM');",
        "  return child;",
        "};",
        "syncBuiltinESMExports();",
      ].join("\n"),
    );
    const childScript = [
      "setTimeout(() => process.exit(0), 300);",
      "setInterval(() => {}, 100);",
    ].join(" ");
    const wrapper = spawnWrapper(root, process.execPath, ["-e", childScript], {
      env: {
        ...process.env,
        NODE_OPTIONS: `--import=${pathToFileURL(preloadPath).href}`,
      },
    });
    t.after(async () => {
      if (wrapper.exitCode === null && wrapper.signalCode === null) {
        wrapper.kill("SIGKILL");
      }
      await cleanupRepoProcesses(root);
      await rm(root, { recursive: true, force: true });
    });

    const exit = await waitForExit(wrapper);
    assert.deepEqual(exit, { code: null, signal: "SIGTERM" });
    await waitUntil(
      async () => (await readLeases(root)).length === 0,
      "spawn-window signal lease cleanup",
    );
  },
);

test(
  "SIGKILL of the wrapper leaves an active lease until its child exits",
  { skip: process.platform === "win32" },
  async (t) => {
    const root = await makeRepo("version = 4\n");
    const readyPath = path.join(root, "child-ready");
    const childScript = [
      "const fs = require('node:fs');",
      `fs.writeFileSync(${JSON.stringify(readyPath)}, 'ready');`,
      "setTimeout(() => process.exit(0), 1_000);",
      "setInterval(() => {}, 100);",
    ].join(" ");
    const wrapper = spawnWrapper(root, process.execPath, ["-e", childScript]);
    t.after(async () => {
      if (wrapper.exitCode === null && wrapper.signalCode === null) {
        wrapper.kill("SIGKILL");
      }
      await cleanupRepoProcesses(root);
      await rm(root, { recursive: true, force: true });
    });

    const lease = await waitForLease(root, (metadata) => metadata.state === "running");
    await waitUntil(async () => {
      try {
        await access(readyPath);
        return true;
      } catch {
        return false;
      }
    }, "child readiness marker");

    const exit = waitForExit(wrapper);
    wrapper.kill("SIGKILL");
    assert.deepEqual(await exit, { code: null, signal: "SIGKILL" });
    const metadata = JSON.parse(await readFile(lease.leasePath, "utf8"));
    assert.equal(probeBuildProcessTree(metadata).state, "active");

    await waitUntil(
      () => probeBuildProcessTree(metadata).state === "empty",
      "orphaned child process to exit",
    );
    assert.equal(finalizeLeaseAfterTreeExit(lease.leasePath), true);
    await assert.rejects(access(lease.leasePath), { code: "ENOENT" });
  },
);

test(
  "lease remains while a grandchild writes after its parent exits",
  { skip: process.platform === "win32" },
  async (t) => {
    const root = await makeRepo("version = 4\n");
    const outputPath = path.join(root, "grandchild-output");
    const grandchildScript = [
      "const fs = require('node:fs');",
      "let count = 0;",
      `const output = ${JSON.stringify(outputPath)};`,
      "const timer = setInterval(() => {",
      "fs.appendFileSync(output, `tick-${count}\\n`);",
      "count += 1;",
      "if (count === 8) { clearInterval(timer); process.exit(0); }",
      "}, 100);",
    ].join(" ");
    const parentScript = [
      "const { spawn } = require('node:child_process');",
      `const child = spawn(process.execPath, ['-e', ${JSON.stringify(grandchildScript)}], { stdio: 'ignore' });`,
      "child.unref();",
      "process.exit(0);",
    ].join(" ");
    const wrapper = spawnWrapper(root, process.execPath, ["-e", parentScript]);
    t.after(async () => {
      if (wrapper.exitCode === null && wrapper.signalCode === null) {
        wrapper.kill("SIGKILL");
      }
      await cleanupRepoProcesses(root);
      await rm(root, { recursive: true, force: true });
    });

    const exit = await waitForExit(wrapper);
    assert.deepEqual(exit, { code: 0, signal: null });
    const lease = await waitForLease(root, (metadata) => metadata.state === "orphaned");
    assert.equal(probeBuildProcessTree(lease.metadata).state, "active");

    await waitUntil(async () => {
      try {
        return (await stat(outputPath)).size > 0;
      } catch {
        return false;
      }
    }, "first grandchild write");
    const firstSize = (await stat(outputPath)).size;
    await new Promise((resolve) => setTimeout(resolve, 250));
    assert.ok((await stat(outputPath)).size > firstSize);

    await waitUntil(
      () => probeBuildProcessTree(lease.metadata).state === "empty",
      "grandchild process group to become empty",
    );
    assert.equal(finalizeLeaseAfterTreeExit(lease.leasePath), true);
    await assert.rejects(access(lease.leasePath), { code: "ENOENT" });
  },
);
