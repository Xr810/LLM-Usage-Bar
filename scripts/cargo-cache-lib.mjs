import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  accessSync,
  closeSync,
  constants,
  existsSync,
  fsyncSync,
  mkdirSync,
  openSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { platform } from "node:os";
import path from "node:path";

const SHELL_WRAPPER_EXTENSIONS = new Set([".bat", ".cmd", ".ps1"]);
let cachedBootIdentity;

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

function isExecutableFile(candidate) {
  try {
    if (!statSync(candidate).isFile()) return false;
    if (process.platform !== "win32") accessSync(candidate, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function rejectShellWrapper(command) {
  if (SHELL_WRAPPER_EXTENSIONS.has(path.extname(command).toLowerCase())) {
    throw new Error(`shell wrapper commands are not supported: ${command}`);
  }
}

function resolveExecutable(command, cwd, env = process.env) {
  rejectShellWrapper(command);

  const hasPath = path.isAbsolute(command) || /[\\/]/.test(command);
  if (hasPath) {
    const candidate = path.resolve(cwd, command);
    if (!isExecutableFile(candidate)) {
      throw new Error(`executable not found: ${command}`);
    }
    return candidate;
  }

  const searchPath = env.PATH ?? env.Path ?? "";
  const suffixes = process.platform === "win32"
    ? path.extname(command)
      ? [""]
      : [".exe", ".com", ""]
    : [""];

  for (const directory of searchPath.split(path.delimiter)) {
    if (!directory) continue;
    for (const suffix of suffixes) {
      const candidate = path.join(directory, `${command}${suffix}`);
      if (isExecutableFile(candidate)) return candidate;
    }
  }

  throw new Error(`executable not found on PATH: ${command}`);
}

function findTauriPackage(cwd) {
  let cursor = path.resolve(cwd);

  while (true) {
    const packageJson = path.join(
      cursor,
      "node_modules",
      "@tauri-apps",
      "cli",
      "package.json",
    );
    if (existsSync(packageJson)) return packageJson;
    const parent = path.dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }

  throw new Error("@tauri-apps/cli is not installed for this workspace");
}

function isWithinDirectory(directory, candidate) {
  const relative = path.relative(directory, candidate);
  return (
    relative === "" ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative))
  );
}

function resolveTauriBin(cwd) {
  const discoveredPackageJson = findTauriPackage(cwd);
  let packageRoot;
  let packageJson;
  try {
    packageRoot = realpathSync(path.dirname(discoveredPackageJson));
    packageJson = realpathSync(discoveredPackageJson);
  } catch {
    throw new Error("@tauri-apps/cli package root is not readable");
  }
  if (
    !statSync(packageRoot).isDirectory() ||
    !isWithinDirectory(packageRoot, packageJson)
  ) {
    throw new Error("@tauri-apps/cli package metadata escapes its real package root");
  }

  const metadata = JSON.parse(readFileSync(packageJson, "utf8"));
  const relativeBin = typeof metadata.bin === "string"
    ? metadata.bin
    : metadata.bin?.tauri;

  if (typeof relativeBin !== "string" || path.isAbsolute(relativeBin)) {
    throw new Error("@tauri-apps/cli does not declare a safe tauri bin");
  }

  let executable;
  try {
    executable = realpathSync(path.resolve(packageRoot, relativeBin));
  } catch {
    throw new Error("@tauri-apps/cli tauri bin is missing");
  }
  if (!isWithinDirectory(packageRoot, executable)) {
    throw new Error("@tauri-apps/cli tauri bin escapes its real package root");
  }
  if (!statSync(executable).isFile()) {
    throw new Error("@tauri-apps/cli tauri bin must be a regular file");
  }
  return executable;
}

export function resolveBuildCommand(command, args = [], cwd = process.cwd()) {
  if (typeof command !== "string" || command.length === 0) {
    throw new TypeError("build command must be a non-empty string");
  }
  if (!Array.isArray(args) || args.some((argument) => typeof argument !== "string")) {
    throw new TypeError("build command arguments must be strings");
  }

  if (command === "tauri" || command === "@tauri-apps/cli") {
    return {
      executable: process.execPath,
      args: [resolveTauriBin(cwd), ...args],
    };
  }

  return {
    executable: resolveExecutable(command, cwd),
    args: [...args],
  };
}

function powershellJson(script, env = process.env) {
  try {
    const output = execFileSync(
      "powershell.exe",
      ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script],
      { encoding: "utf8", env, windowsHide: true },
    ).trim();
    return output ? JSON.parse(output) : [];
  } catch {
    return null;
  }
}

function isValidWindowsBootTime(value) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,7}))?Z$/.exec(
    value,
  );
  if (!match) return false;

  const [, yearText, monthText, dayText, hourText, minuteText, secondText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  if (
    year < 1601 ||
    month < 1 ||
    month > 12 ||
    day < 1 ||
    hour > 23 ||
    minute > 59 ||
    second > 59
  ) {
    return false;
  }

  const calendarDate = new Date(Date.UTC(year, month - 1, day));
  return (
    calendarDate.getUTCFullYear() === year &&
    calendarDate.getUTCMonth() === month - 1 &&
    calendarDate.getUTCDate() === day
  );
}

function classifyBootIdentity(identity) {
  if (typeof identity !== "string" || identity.length === 0) return "invalid";
  if (identity === `${process.platform}:unknown`) return "unknown";

  if (process.platform === "linux") {
    return /^linux:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(
      identity,
    )
      ? "known"
      : "invalid";
  }

  if (process.platform === "darwin") {
    const match = /^darwin:(\d+):(\d{1,6})$/.exec(identity);
    const seconds = match ? Number(match[1]) : Number.NaN;
    const microseconds = match ? Number(match[2]) : Number.NaN;
    return match &&
      Number.isSafeInteger(seconds) &&
      seconds > 0 &&
      Number.isSafeInteger(microseconds) &&
      microseconds < 1_000_000
      ? "known"
      : "invalid";
  }

  if (process.platform === "win32") {
    return identity.startsWith("win32:") &&
      isValidWindowsBootTime(identity.slice("win32:".length))
      ? "known"
      : "invalid";
  }

  return "invalid";
}

export function currentBootIdentity() {
  if (cachedBootIdentity) return cachedBootIdentity;

  if (process.platform === "linux") {
    try {
      const candidate = `linux:${readFileSync(
        "/proc/sys/kernel/random/boot_id",
        "utf8",
      ).trim().toLowerCase()}`;
      cachedBootIdentity = classifyBootIdentity(candidate) === "known"
        ? candidate
        : "linux:unknown";
      return cachedBootIdentity;
    } catch {
      cachedBootIdentity = "linux:unknown";
      return cachedBootIdentity;
    }
  }

  if (process.platform === "darwin") {
    try {
      const bootTime = execFileSync("sysctl", ["-n", "kern.boottime"], {
        encoding: "utf8",
      }).trim();
      const match = /\bsec\s*=\s*(\d+),\s*usec\s*=\s*(\d+)\b/.exec(bootTime);
      const candidate = match
        ? `darwin:${BigInt(match[1])}:${Number(match[2])}`
        : "darwin:unknown";
      cachedBootIdentity = classifyBootIdentity(candidate) === "known"
        ? candidate
        : "darwin:unknown";
      return cachedBootIdentity;
    } catch {
      cachedBootIdentity = "darwin:unknown";
      return cachedBootIdentity;
    }
  }

  if (process.platform === "win32") {
    const bootTime = powershellJson(
      "(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o') | ConvertTo-Json -Compress",
    );
    const candidate = typeof bootTime === "string"
      ? `win32:${bootTime}`
      : "win32:unknown";
    cachedBootIdentity = classifyBootIdentity(candidate) === "known"
      ? candidate
      : "win32:unknown";
    return cachedBootIdentity;
  }

  cachedBootIdentity = `${platform()}:unknown`;
  return cachedBootIdentity;
}

function processIdentity(pid) {
  let startedAtToken = null;

  if (process.platform === "linux") {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const fields = stat.slice(stat.lastIndexOf(")") + 2).trim().split(/\s+/);
      startedAtToken = fields[19] ?? null;
    } catch {
      startedAtToken = null;
    }
  } else if (process.platform === "win32") {
    const result = powershellJson(
      "$target = [uint32]$env:LLM_USAGE_BAR_PID; Get-CimInstance Win32_Process -Filter \"ProcessId = $target\" | ForEach-Object { $_.CreationDate.ToUniversalTime().ToString('o') } | ConvertTo-Json -Compress",
      { ...process.env, LLM_USAGE_BAR_PID: String(pid) },
    );
    startedAtToken = typeof result === "string" ? result : null;
  } else {
    try {
      startedAtToken = execFileSync(
        "ps",
        ["-o", "lstart=", "-p", String(pid)],
        { encoding: "utf8" },
      ).trim() || null;
    } catch {
      startedAtToken = null;
    }
  }

  return { pid, startedAtToken };
}

function processTreeKind() {
  return process.platform === "win32"
    ? "windows-descendants"
    : "unix-process-group";
}

export function pendingLease(wrapperPid, cwd, command) {
  return {
    version: 1,
    state: "pending",
    startedAt: new Date().toISOString(),
    bootIdentity: currentBootIdentity(),
    cwd,
    command: [...command],
    wrapper: processIdentity(wrapperPid),
    child: null,
    processTree: {
      kind: processTreeKind(),
      processGroupId: null,
      safetyState: "pending",
    },
  };
}

export function runningLease({
  wrapperPid,
  childPid,
  processGroupId,
  bootIdentity = currentBootIdentity(),
  cwd,
  command,
}) {
  return {
    version: 1,
    state: "running",
    startedAt: new Date().toISOString(),
    bootIdentity,
    cwd,
    command: [...command],
    wrapper: processIdentity(wrapperPid),
    child: processIdentity(childPid),
    processTree: {
      kind: processTreeKind(),
      processGroupId,
      safetyState: "active",
    },
  };
}

function syncDirectory(directory) {
  let descriptor;
  try {
    descriptor = openSync(directory, constants.O_RDONLY);
    fsyncSync(descriptor);
  } catch {
    // Some platforms do not support fsync on directory descriptors.
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

export function writeLeaseAtomic(leasePath, metadata) {
  const directory = path.dirname(leasePath);
  mkdirSync(directory, { recursive: true });
  const temporary = path.join(
    directory,
    `.${path.basename(leasePath)}.${process.pid}.${randomUUID()}.tmp`,
  );
  let descriptor;

  try {
    descriptor = openSync(temporary, "wx", 0o600);
    writeFileSync(descriptor, `${JSON.stringify(metadata, null, 2)}\n`, "utf8");
    fsyncSync(descriptor);
    closeSync(descriptor);
    descriptor = undefined;
    renameSync(temporary, leasePath);
    syncDirectory(directory);
  } catch (error) {
    if (descriptor !== undefined) closeSync(descriptor);
    rmSync(temporary, { force: true });
    throw error;
  }
}

export function removeLease(leasePath) {
  rmSync(leasePath, { force: true });
}

function windowsProcessSnapshot() {
  const result = powershellJson(
    "$items = @(Get-CimInstance Win32_Process | ForEach-Object { [pscustomobject]@{ pid = [uint32]$_.ProcessId; parentPid = [uint32]$_.ParentProcessId; startedAtToken = if ($_.CreationDate) { $_.CreationDate.ToUniversalTime().ToString('o') } else { $null } } }); $items | ConvertTo-Json -Compress",
  );
  if (result === null) return null;
  const items = Array.isArray(result) ? result : [result];
  if (
    items.some(
      (item) =>
        !Number.isSafeInteger(item?.pid) ||
        !Number.isSafeInteger(item?.parentPid),
    )
  ) {
    return null;
  }
  return items;
}

export function probeWindowsProcessTree(lease, processes) {
  if (lease.processTree?.kind !== "windows-descendants") {
    return { state: "unknown", reason: "unsupported-process-tree-kind" };
  }

  const childPid = lease.child?.pid;
  if (!Number.isSafeInteger(childPid) || childPid <= 0) {
    return { state: "unknown", reason: "missing-child-identity" };
  }

  if (!Array.isArray(processes)) {
    return { state: "unknown", reason: "invalid-windows-process-snapshot" };
  }

  const root = processes.find((process) => process.pid === childPid);
  if (root) {
    const expectedStart = lease.child.startedAtToken;
    if (
      expectedStart &&
      root.startedAtToken &&
      expectedStart !== root.startedAtToken
    ) {
      return { state: "unknown", reason: "child-pid-reused" };
    }
    return { state: "active", reason: "child-process-active" };
  }

  return { state: "unknown", reason: "windows-root-missing" };
}

function probeWindowsTree(lease) {
  const processes = windowsProcessSnapshot();
  if (!processes) {
    return { state: "unknown", reason: "windows-enumeration-unavailable" };
  }
  return probeWindowsProcessTree(lease, processes);
}

export function probeBuildProcessTree(lease) {
  if (!lease || typeof lease !== "object") {
    return { state: "unknown", reason: "invalid-lease" };
  }

  const bootIdentity = currentBootIdentity();
  const leaseBootState = classifyBootIdentity(lease.bootIdentity);
  const currentBootState = classifyBootIdentity(bootIdentity);
  if (leaseBootState === "invalid") {
    return { state: "unknown", reason: "invalid-boot-identity" };
  }
  if (leaseBootState === "unknown" || currentBootState !== "known") {
    return { state: "unknown", reason: "boot-identity-unavailable" };
  }
  if (lease.bootIdentity !== bootIdentity) {
    return { state: "empty", reason: "previous-boot" };
  }

  if (process.platform === "win32") return probeWindowsTree(lease);
  if (lease.processTree?.kind !== "unix-process-group") {
    return { state: "unknown", reason: "unsupported-process-tree-kind" };
  }

  const processGroupId = lease.processTree.processGroupId;
  if (!Number.isSafeInteger(processGroupId) || processGroupId <= 0) {
    return { state: "unknown", reason: "missing-process-group" };
  }

  try {
    process.kill(-processGroupId, 0);
    return { state: "active", reason: "process-group-active" };
  } catch (error) {
    if (error.code === "ESRCH") {
      return { state: "empty", reason: "process-group-empty" };
    }
    if (error.code === "EPERM") {
      return { state: "active", reason: "process-group-not-owned" };
    }
    return { state: "unknown", reason: `process-group-probe-${error.code ?? "failed"}` };
  }
}

export function finalizeLeaseAfterTreeExit(leasePath) {
  let lease;
  try {
    lease = JSON.parse(readFileSync(leasePath, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return true;
    return false;
  }

  const probe = probeBuildProcessTree(lease);
  if (probe.state === "empty") {
    removeLease(leasePath);
    return true;
  }

  writeLeaseAtomic(leasePath, {
    ...lease,
    state: "orphaned",
    orphanedAt: lease.orphanedAt ?? new Date().toISOString(),
    processTree: {
      ...lease.processTree,
      safetyState: probe.state,
    },
    lastProbe: probe,
  });
  return false;
}
