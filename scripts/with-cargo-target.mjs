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

const [command, ...args] = process.argv.slice(2);
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

let child;
try {
  child = spawn(invocation.executable, invocation.args, {
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: resolved.targetDir },
    shell: false,
    detached: process.platform !== "win32",
  });
} catch (error) {
  removeLease(leasePath);
  console.error(error.message);
  process.exit(1);
}

const childPid = child.pid;
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

const signalHandlers = new Map();
let forwardedSignal = null;
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  const handler = () => {
    forwardedSignal ??= signal;
    try {
      if (process.platform !== "win32" && Number.isSafeInteger(childPid)) {
        process.kill(-childPid, signal);
      } else {
        child.kill(signal);
      }
    } catch (error) {
      if (error.code !== "ESRCH") console.error(error.message);
    }
  };
  signalHandlers.set(signal, handler);
  process.on(signal, handler);
}

let finished = false;
function removeSignalHandlers() {
  for (const [signal, handler] of signalHandlers) {
    process.off(signal, handler);
  }
}

child.on("error", (error) => {
  if (finished) return;
  finished = true;
  removeSignalHandlers();
  if (Number.isSafeInteger(childPid)) finalizeLeaseAfterTreeExit(leasePath);
  else removeLease(leasePath);
  console.error(error.message);
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
