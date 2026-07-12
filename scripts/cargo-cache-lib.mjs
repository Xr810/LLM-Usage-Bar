import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";

export function sha256File(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function gitPath(cwd, argument) {
  return execFileSync(
    "git",
    ["rev-parse", "--path-format=absolute", argument],
    {
      cwd,
      encoding: "utf8",
    },
  ).trim();
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
