import test from "node:test";
import assert from "node:assert/strict";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import {
  currentBootIdentity,
  finalizeLeaseAfterTreeExit,
  probeBuildProcessTree,
  removeLease,
  resolveBuildCommand,
  resolveCargoTarget,
  writeLeaseAtomic,
} from "./cargo-cache-lib.mjs";

const wrapperPath = path.resolve("scripts/with-cargo-target.mjs");

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

function spawnWrapper(root, command, args) {
  return spawn(process.execPath, [wrapperPath, command, ...args], {
    cwd: root,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

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
  assert.equal(tauri.args[0], path.join(packageDir, "tauri.js"));
  assert.deepEqual(tauri.args.slice(1), ["dev"]);

  const batch = path.join(fixture, "cargo.cmd");
  await writeFile(batch, "@echo off\r\n");
  await chmod(batch, 0o755);
  assert.throws(
    () => resolveBuildCommand(batch, [], fixture),
    /shell wrapper/i,
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
  for (const bootIdentity of ["malformed", "another-platform:known-boot"]) {
    const probe = probeBuildProcessTree({
      version: 1,
      state: "orphaned",
      bootIdentity,
      processTree: { kind: "unknown" },
    });

    assert.equal(probe.state, "unknown");
  }
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

test("lease is removed after a successful child exit", async (t) => {
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
  assert.deepEqual(await readdir(path.join(targetDir, ".active")), []);
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
