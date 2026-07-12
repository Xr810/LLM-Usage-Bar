import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
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
  await writeFile(path.join(root, "src-tauri", "Cargo.lock"), lockText);
  git(root, "add", ".");
  git(root, "commit", "-m", "fixture");
  return root;
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
