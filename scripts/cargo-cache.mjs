#!/usr/bin/env node

import {
  applyPrunePlan,
  createPruneSnapshot,
  planPrune,
  probeBuildProcessTree,
} from "./cargo-cache-lib.mjs";

function usage() {
  return "Usage: node scripts/cargo-cache.mjs status | prune [--apply]";
}

function printPlan(plan, dryRun) {
  if (dryRun) console.log("Cargo cache prune dry run (no files deleted)");
  console.log(`keep: ${plan.keep.length}`);
  console.log(`remove: ${plan.remove.length}`);
  for (const bucket of plan.remove) console.log(`  remove ${bucket.hash}`);
  for (const reason of plan.unsafeReasons) console.log(`  unsafe ${reason}`);
}

function main(argv) {
  const [command, ...options] = argv;
  if (!new Set(["status", "prune"]).has(command)) {
    throw new Error(usage());
  }
  if (options.some((option) => option !== "--apply")) {
    throw new Error(usage());
  }
  const apply = options.includes("--apply");
  if (command === "status" && apply) throw new Error(usage());
  const snapshot = createPruneSnapshot(process.cwd());
  const plan = planPrune({
    referencedHashes: snapshot.referencedHashes,
    buckets: snapshot.buckets,
    probeBuildProcessTree,
  });
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
