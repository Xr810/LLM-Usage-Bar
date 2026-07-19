#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  applyPrunePlan,
  createPruneSnapshot,
  planPrune,
  probeBuildProcessTree,
  resolveCargoTarget,
} from "./cargo-cache-lib.mjs";

const wrapperPath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "with-cargo-target.mjs",
);

function usage() {
  return "Usage: node scripts/cargo-cache.mjs status | prune | clean-current [--apply]";
}

function printPlan(plan, dryRun) {
  if (dryRun) console.log("Cargo cache prune dry run (no files deleted)");
  console.log(`keep: ${plan.keep.length}`);
  console.log(`remove: ${plan.remove.length}`);
  for (const bucket of plan.remove) console.log(`  remove ${bucket.hash}`);
  for (const reason of plan.unsafeReasons) console.log(`  unsafe ${reason}`);
}

function cleanCurrentCache(apply) {
  const { lockHash, lockfile } = resolveCargoTarget();
  const manifestPath = path.join(path.dirname(lockfile), "Cargo.toml");
  if (!apply) {
    console.log("Cargo current-cache clean dry run (no files deleted)");
    console.log(`current lock: ${lockHash}`);
    console.log("would run: pnpm rust -- clean --manifest-path src-tauri/Cargo.toml");
    return;
  }

  const result = spawnSync(
    process.execPath,
    [wrapperPath, "cargo", "clean", "--manifest-path", manifestPath],
    { cwd: process.cwd(), stdio: "inherit" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) process.exitCode = result.status ?? 1;
}

function main(argv) {
  const [command, ...options] = argv[0] === "--" ? argv.slice(1) : argv;
  if (!new Set(["status", "prune", "clean-current"]).has(command)) {
    throw new Error(usage());
  }
  if (options.some((option) => option !== "--apply")) {
    throw new Error(usage());
  }
  const apply = options.includes("--apply");
  if (command === "status" && apply) throw new Error(usage());
  if (command === "clean-current") {
    cleanCurrentCache(apply);
    return;
  }
  const snapshot = createPruneSnapshot(process.cwd());
  const plan = planPrune({
    referencedHashes: snapshot.referencedHashes,
    buckets: snapshot.buckets,
    probeBuildProcessTree,
  });
  if (command === "status") {
    console.log(`current lock: ${resolveCargoTarget().lockHash}`);
    for (const hash of [...snapshot.referencedHashes].sort()) {
      console.log(`referenced lock: ${hash}`);
    }
  }
  printPlan(plan, command === "prune" && !apply);
  if (!apply) return;

  const result = applyPrunePlan({
    cwd: process.cwd(),
    plannedBuckets: plan.remove,
    probeBuildProcessTree,
  });
  for (const hash of result.deleted) console.log(`deleted ${hash}`);
  if (!result.ok) {
    const deleted = result.deleted.length > 0
      ? `; already deleted: ${result.deleted.join(", ")}`
      : "";
    console.error(`cargo-cache: ${result.error}${deleted}`);
    process.exitCode = 1;
  }
}

try {
  main(process.argv.slice(2));
} catch (error) {
  console.error(`cargo-cache: ${error.message}`);
  process.exitCode = 1;
}
