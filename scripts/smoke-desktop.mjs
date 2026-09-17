// Run a packaged desktop executable against an empty, temporary data directory.
// A response from its loopback router proves the native startup and DB migrations
// completed. No real CLI configuration or credentials are used.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const executable = process.argv[2];
if (!executable)
  throw new Error("Usage: node scripts/smoke-desktop.mjs <executable>");
const testDirectory = await mkdtemp(path.join(tmpdir(), "llm-release-smoke-"));
const app = spawn(path.resolve(executable), [], {
  env: { ...process.env, LLM_USAGE_BAR_TEST_HOME: testDirectory },
  stdio: ["ignore", "pipe", "pipe"],
});
let output = "";
app.stdout.on("data", (chunk) => {
  output = (output + chunk).slice(-12000);
});
app.stderr.on("data", (chunk) => {
  output = (output + chunk).slice(-12000);
});
let spawnError;
app.on("error", (error) => {
  spawnError = error;
});
try {
  let ready = false;
  for (let attempt = 0; attempt < 60; attempt++) {
    if (spawnError) throw spawnError;
    if (app.exitCode !== null)
      throw new Error(`App exited (${app.exitCode}): ${output}`);
    try {
      const response = await fetch("http://127.0.0.1:8788/v1/responses", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: "release-smoke-unconfigured",
          input: "test",
          stream: false,
        }),
        signal: AbortSignal.timeout(1000),
      });
      if (response.status === 503) {
        ready = true;
        break;
      }
    } catch {
      /* Startup is asynchronous. */
    }
    await delay(500);
  }
  assert.ok(ready, `App router did not become ready: ${output}`);
  const database = path.join(
    testDirectory,
    ".llm-usage-bar",
    "llm-usage-bar.db",
  );
  const header = (await readFile(database)).subarray(0, 16).toString();
  assert.equal(header, "SQLite format 3\0");
  await delay(2000);
  assert.equal(app.exitCode, null, `App exited after startup: ${output}`);
  console.log(
    "Desktop executable started, initialized its database, and served its local router.",
  );
} finally {
  app.kill();
}
