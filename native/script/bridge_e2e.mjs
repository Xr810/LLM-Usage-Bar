#!/usr/bin/env node

import { spawn } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import net from "node:net";
import path from "node:path";
import process from "node:process";

const executable = process.argv[2] ?? process.env.LLM_USAGE_BAR_BRIDGE_EXECUTABLE;
if (!executable) {
  throw new Error("usage: bridge_e2e.mjs /absolute/path/to/llm-usage-bar");
}

const testHome = mkdtempSync(path.join("/private/tmp", "lub-bridge-"));
chmodSync(testHome, 0o700);
const runtimeDirectory = path.join(testHome, ".llm-usage-bar", "runtime");
const socketPath = path.join(runtimeDirectory, "native-bridge-v1.sock");
const settingsDirectory = path.join(testHome, ".llm-usage-bar");
const recognizableSecrets = [
  "E2E_RECOGNIZABLE_WEBDAV_PASSWORD",
  "E2E_RECOGNIZABLE_ACCESS_KEY_ID",
  "E2E_RECOGNIZABLE_S3_SECRET",
  "E2E_RECOGNIZABLE_BACKUP_SECRET",
];
const forbiddenWireFields = [
  "webdavSync",
  "baseUrl",
  "username",
  "password",
  "s3Sync",
  "bucket",
  "accessKeyId",
  "secretAccessKey",
  "webdavBackup",
  "authorization",
];
mkdirSync(settingsDirectory, { recursive: true, mode: 0o700 });
writeFileSync(
  path.join(settingsDirectory, "settings.json"),
  JSON.stringify({
    webdavSync: {
      baseUrl: "https://dav.secret.invalid",
      username: "e2e-user",
      password: recognizableSecrets[0],
    },
    s3Sync: {
      region: "test-region",
      bucket: "secret-bucket",
      accessKeyId: recognizableSecrets[1],
      secretAccessKey: recognizableSecrets[2],
    },
    webdavBackup: { authorization: recognizableSecrets[3] },
  }),
  { mode: 0o600 },
);
const child = spawn(executable, ["--native-bridge-server"], {
  env: {
    ...process.env,
    LLM_USAGE_BAR_TEST_HOME: testHome,
    LLM_USAGE_BAR_NATIVE_BRIDGE_ISOLATED_TEST: "1",
  },
  stdio: ["ignore", "pipe", "pipe"],
});

let processOutput = "";
for (const stream of [child.stdout, child.stderr]) {
  stream.setEncoding("utf8");
  stream.on("data", (chunk) => {
    processOutput = `${processOutput}${chunk}`.slice(-16_384);
  });
}

let nextId = 0;
const wireLines = [];

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function mode(file) {
  return statSync(file).mode & 0o777;
}

function wait(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitForSocket() {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      if (mode(runtimeDirectory) !== 0o700 || mode(socketPath) !== 0o600) {
        await wait(25);
        continue;
      }
      const client = await NDJSONClient.connect(socketPath);
      client.close();
      return;
    } catch {
      if (child.exitCode !== null) {
        throw new Error(`bridge exited before creating its socket\n${processOutput}`);
      }
      await wait(100);
    }
  }
  throw new Error(`timed out waiting for ${socketPath}\n${processOutput}`);
}

class NDJSONClient {
  constructor(socket) {
    this.socket = socket;
    this.buffer = "";
    this.pending = [];
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      this.buffer += chunk;
      while (this.buffer.includes("\n")) {
        const newline = this.buffer.indexOf("\n");
        const line = this.buffer.slice(0, newline);
        this.buffer = this.buffer.slice(newline + 1);
        wireLines.push(line);
        this.pending.shift()?.resolve(JSON.parse(line));
      }
    });
    socket.on("error", (error) => this.rejectAll(error));
    socket.on("close", () => this.rejectAll(new Error("bridge socket closed")));
  }

  static connect(socketPath) {
    return new Promise((resolve, reject) => {
      const socket = net.createConnection({ path: socketPath });
      socket.once("connect", () => resolve(new NDJSONClient(socket)));
      socket.once("error", reject);
    });
  }

  request(method, params, protocolVersion = 1) {
    const id = `e2e-${++nextId}`;
    const request = { protocolVersion, id, method };
    if (params !== undefined) request.params = params;
    const requestLine = JSON.stringify(request);
    wireLines.push(requestLine);
    return new Promise((resolve, reject) => {
      this.pending.push({ resolve, reject });
      this.socket.write(`${requestLine}\n`);
    }).then((response) => {
      assert(response.protocolVersion === 1, `${method}: unexpected protocol`);
      assert(response.id === id, `${method}: response id mismatch`);
      return response;
    });
  }

  async hello() {
    const response = await this.request("hello");
    assert(response.result?.readOnly === false, "hello still advertised read-only mode");
    assert(
      response.result?.mutationSchemaVersion === 1,
      "hello mutation schema mismatch",
    );
    return response;
  }

  close() {
    this.socket.destroy();
  }

  rejectAll(error) {
    for (const pending of this.pending.splice(0)) pending.reject(error);
  }
}

async function expectIncompleteFrame() {
  const socket = net.createConnection({ path: socketPath });
  socket.setEncoding("utf8");
  const response = new Promise((resolve, reject) => {
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += chunk;
      const newline = buffer.indexOf("\n");
      if (newline >= 0) resolve(JSON.parse(buffer.slice(0, newline)));
    });
    socket.on("error", reject);
  });
  await new Promise((resolve, reject) => {
    socket.once("connect", resolve);
    socket.once("error", reject);
  });
  socket.end('{"protocolVersion":1,"id":"half","method":"hello"}');
  const message = await response;
  assert(message.error?.code === "incomplete_request", "half packet was not rejected safely");
}

async function waitForExit() {
  if (child.exitCode !== null) return child.exitCode;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("bridge did not exit after shutdown")), 10_000);
    child.once("exit", (code) => {
      clearTimeout(timer);
      resolve(code);
    });
  });
}

try {
  await waitForSocket();
  const runtimeMode = mode(runtimeDirectory);
  const socketMode = mode(socketPath);
  assert(runtimeMode === 0o700, `runtime directory mode is ${runtimeMode.toString(8)}, not 0700`);
  assert(socketMode === 0o600, `socket mode is ${socketMode.toString(8)}, not 0600`);

  const client = await NDJSONClient.connect(socketPath);
  let response = await client.request("getRuntimeStatus");
  assert(response.error?.code === "hello_required", "request before hello was accepted");
  response = await client.request("hello", undefined, 99);
  assert(response.error?.code === "protocol_mismatch", "protocol mismatch was accepted");
  await client.hello();

  response = await client.request("capabilities");
  for (const method of [
    "getProviderDashboard",
    "getModelDashboard",
    "getAgentBreakdown",
    "getUsageEvents",
    "getNativeSettings",
    "setNativeSettings",
  ]) {
    assert(response.result.methods.includes(method), `missing capability ${method}`);
  }
  assert(
    response.result.mutations.includes("setNativeSettings"),
    "settings mutation was not advertised",
  );

  const initialSettings = await client.request("getNativeSettings");
  assert(initialSettings.result?.schemaVersion === 1, "native settings schema mismatch");
  assert(
    /^[0-9a-f]{32}$/.test(initialSettings.result?.revision ?? ""),
    "native settings revision is not a stable digest",
  );
  const initialRevision = initialSettings.result.revision;
  const savedSettings = await client.request("setNativeSettings", {
    expectedRevision: initialRevision,
    patch: {
      silentStartup: true,
      language: "ja",
      sharedApiDailyBudgetUsd: "7.25",
      usageDashboardRefreshIntervalMs: 5000,
    },
  });
  assert(savedSettings.result?.data.silentStartup === true, "silent startup was not saved");
  assert(savedSettings.result?.data.language === "ja", "language was not saved");
  assert(
    savedSettings.result?.data.sharedApiDailyBudgetUsd === "7.25",
    "decimal budget was not saved as a string",
  );
  assert(
    savedSettings.result?.revision !== initialRevision,
    "settings revision did not change",
  );

  const reloadedSettings = await client.request("getNativeSettings");
  assert(
    reloadedSettings.result?.revision === savedSettings.result.revision,
    "get after set returned a different revision",
  );
  assert(reloadedSettings.result?.data.language === "ja", "get after set lost the patch");

  const conflict = await client.request("setNativeSettings", {
    expectedRevision: initialRevision,
    patch: { showInTray: false },
  });
  assert(conflict.error?.code === "settings_conflict", "stale revision did not conflict");
  assert(
    conflict.error?.data.revision === reloadedSettings.result.revision,
    "conflict did not carry the fresh revision",
  );
  assert(
    conflict.error?.data.data.language === "ja",
    "conflict did not carry the fresh projection",
  );

  const invalid = await client.request("setNativeSettings", {
    expectedRevision: reloadedSettings.result.revision,
    patch: { language: "fr" },
  });
  assert(invalid.error?.code === "invalid_setting", "invalid setting was accepted");
  assert(invalid.error?.data.key === "language", "invalid setting did not name its key");

  response = await client.request("getRuntimeStatus");
  assert(response.result.bridgeOnly === true, "runtime is not bridge-only");
  assert(response.result.databaseOwner === "rust", "database owner is not Rust");
  assert(response.result.schedulerOwner === "rust", "scheduler owner is not Rust");

  const endAt = Math.floor(Date.now() / 1000);
  const range = { startAt: endAt - 86_400, endAt };
  const providerResponse = await client.request("getProviderDashboard", range);
  assert(providerResponse.result?.schemaVersion === 1, "provider dashboard schema mismatch");
  assert(Array.isArray(providerResponse.result.data.providers), "provider rows are missing");
  const modelResponse = await client.request("getModelDashboard", range);
  assert(modelResponse.result?.schemaVersion === 1, "model dashboard schema mismatch");
  const agentResponse = await client.request("getAgentBreakdown", range);
  assert(agentResponse.result?.schemaVersion === 1, "agent dashboard schema mismatch");

  const providerId = providerResponse.result.data.providers[0]?.provider?.id;
  if (providerId) {
    const eventsResponse = await client.request("getUsageEvents", {
      providerId,
      ...range,
      page: 1,
      pageSize: 50,
    });
    assert(eventsResponse.result?.schemaVersion === 1, "usage event schema mismatch");
  }

  const concurrentClients = await Promise.all([
    NDJSONClient.connect(socketPath),
    NDJSONClient.connect(socketPath),
  ]);
  await Promise.all(concurrentClients.map((concurrent) => concurrent.hello()));
  const statuses = await Promise.all(
    concurrentClients.map((concurrent) => concurrent.request("getRuntimeStatus")),
  );
  assert(statuses.every((status) => status.result.clientCount >= 2), "concurrent clients were not tracked");
  concurrentClients.forEach((concurrent) => concurrent.close());

  await expectIncompleteFrame();
  const wireTranscript = wireLines.join("\n");
  for (const forbidden of [...recognizableSecrets, ...forbiddenWireFields]) {
    assert(!wireTranscript.includes(forbidden), `secret leaked on the wire: ${forbidden}`);
  }
  response = await client.request("shutdown");
  assert(response.result?.accepted === true, "shutdown was not accepted");
  client.close();
  const exitCode = await waitForExit();
  assert(exitCode === 0, `bridge exited with ${exitCode}`);

  console.log("native bridge e2e: passed");
} catch (error) {
  child.kill("SIGTERM");
  console.error(error instanceof Error ? error.message : error);
  if (processOutput) console.error(processOutput);
  process.exitCode = 1;
} finally {
  rmSync(testHome, { recursive: true, force: true });
}
