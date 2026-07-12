#!/usr/bin/env node
import { spawn } from "node:child_process";
import path from "node:path";
import {
  currentBootIdentity,
  finalizeLeaseAfterTreeExit,
  pendingLease,
  removeLease,
  resolveBuildCommand,
  resolveCargoTarget,
  runningLease,
  writeLeaseAtomic,
} from "./cargo-cache-lib.mjs";

const [command, ...rawArgs] = process.argv.slice(2);
const args = rawArgs[0] === "--" ? rawArgs.slice(1) : rawArgs;
if (!command) {
  console.error("usage: with-cargo-target <command> [args...]");
  process.exit(64);
}

let invocation;
let resolved;
try {
  resolved = resolveCargoTarget();
  invocation = resolveBuildCommand(command, args);
} catch (error) {
  console.error(error.message);
  process.exit(127);
}

const activeDir = path.join(resolved.targetDir, ".active");
const leasePath = path.join(activeDir, `${process.pid}-${Date.now()}.json`);
writeLeaseAtomic(
  leasePath,
  pendingLease(process.pid, process.cwd(), [command, ...args]),
);

let child = null;
let childPid = null;
let finished = false;
let forwardedSignal = null;
const signalHandlers = new Map();

function removeSignalHandlers() {
  for (const [signal, handler] of signalHandlers) {
    process.off(signal, handler);
  }
}

function forwardSignalToChild(signal) {
  if (!child || !Number.isSafeInteger(childPid)) return false;
  try {
    if (process.platform !== "win32") process.kill(-childPid, signal);
    else child.kill(signal);
    return true;
  } catch (error) {
    if (error.code !== "ESRCH") console.error(error.message);
    return false;
  }
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  const handler = () => {
    forwardedSignal ??= signal;
    forwardSignalToChild(signal);
  };
  signalHandlers.set(signal, handler);
  process.on(signal, handler);
}

try {
  child = spawn(invocation.executable, invocation.args, {
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: resolved.targetDir },
    shell: false,
    detached: process.platform !== "win32",
  });
} catch (error) {
  removeSignalHandlers();
  removeLease(leasePath);
  console.error(error.message);
  if (forwardedSignal) process.kill(process.pid, forwardedSignal);
  process.exit(1);
}

childPid = child.pid;
if (Number.isSafeInteger(childPid)) {
  writeLeaseAtomic(
    leasePath,
    runningLease({
      wrapperPid: process.pid,
      childPid,
      processGroupId: process.platform === "win32" ? null : childPid,
      bootIdentity: currentBootIdentity(),
      cwd: process.cwd(),
      command: [command, ...args],
    }),
  );
}

child.on("error", (error) => {
  if (finished) return;
  finished = true;
  removeSignalHandlers();
  if (Number.isSafeInteger(childPid)) finalizeLeaseAfterTreeExit(leasePath);
  else removeLease(leasePath);
  console.error(error.message);
  if (forwardedSignal) {
    process.kill(process.pid, forwardedSignal);
    return;
  }
  process.exitCode = 1;
});

child.on("exit", (code, signal) => {
  if (finished) return;
  finished = true;
  finalizeLeaseAfterTreeExit(leasePath);
  removeSignalHandlers();

  const terminationSignal = signal ?? forwardedSignal;
  if (terminationSignal) {
    process.kill(process.pid, terminationSignal);
    return;
  }
  process.exitCode = code ?? 1;
});

if (forwardedSignal) forwardSignalToChild(forwardedSignal);
