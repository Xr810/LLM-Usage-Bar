import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  accessSync,
  closeSync,
  constants,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
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
    worktreeRoot,
    lockfile,
    lockHash,
    targetDir: path.join(commonRoot, ".cache", "cargo-targets", lockHash),
  };
}

export function resolveCargoTargetForInvocation(
  command,
  args = [],
  cwd = process.cwd(),
) {
  const shared = resolveCargoTarget(cwd);
  const isTauriBuild =
    (command === "tauri" || command === "@tauri-apps/cli") &&
    args[0] === "build";

  if (!isTauriBuild) return shared;

  return {
    ...shared,
    sharedTargetDir: shared.targetDir,
    targetDir: path.join(shared.worktreeRoot, "release", "tauri-target"),
  };
}

function parseWorktreePorcelain(output) {
  const worktrees = [];
  for (const field of output.split("\0")) {
    if (field.startsWith("worktree ")) {
      worktrees.push(field.slice("worktree ".length));
    }
  }
  if (worktrees.length === 0) {
    throw new Error("git did not report any worktrees");
  }
  return worktrees;
}

function safeWorktreeLockfile(worktree) {
  let root;
  let lockfile;
  try {
    root = realpathSync(worktree);
    const rootMetadata = lstatSync(root);
    if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
      throw new Error("worktree root is not a real directory");
    }
    lockfile = path.join(root, "src-tauri", "Cargo.lock");
    const lockMetadata = lstatSync(lockfile);
    if (!lockMetadata.isFile() || lockMetadata.isSymbolicLink()) {
      throw new Error("Cargo.lock is not a regular file");
    }
    const resolvedLockfile = realpathSync(lockfile);
    if (!isWithinDirectory(root, resolvedLockfile)) {
      throw new Error("Cargo.lock escapes its worktree");
    }
    return resolvedLockfile;
  } catch (error) {
    throw new Error(
      `cannot safely read worktree Cargo.lock at ${lockfile ?? worktree}: ${error.message}`,
      { cause: error },
    );
  }
}

export function collectReferencedCargoHashes(cwd = process.cwd()) {
  const output = execFileSync(
    "git",
    ["worktree", "list", "--porcelain", "-z"],
    { cwd, encoding: "utf8" },
  );
  const worktrees = parseWorktreePorcelain(output);
  const referencedHashes = new Set();

  for (const worktree of worktrees) {
    const lockfile = safeWorktreeLockfile(worktree);
    referencedHashes.add(sha256File(lockfile));
  }

  return {
    commonRoot: gitCommonRoot(cwd),
    referencedHashes,
    worktrees,
  };
}

function safeCacheRoot(commonRoot) {
  const resolvedCommonRoot = realpathSync(commonRoot);
  const cacheParent = path.join(resolvedCommonRoot, ".cache");
  const cacheRoot = path.join(cacheParent, "cargo-targets");

  for (const [candidate, label] of [
    [cacheParent, "cache parent"],
    [cacheRoot, "Cargo cache root"],
  ]) {
    let metadata;
    try {
      metadata = lstatSync(candidate);
    } catch (error) {
      if (error.code === "ENOENT") return { cacheRoot, exists: false };
      throw error;
    }
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error(`${label} must be a real directory: ${candidate}`);
    }
  }

  const resolvedCacheRoot = realpathSync(cacheRoot);
  if (!isWithinDirectory(resolvedCommonRoot, resolvedCacheRoot)) {
    throw new Error(`Cargo cache root escapes the repository: ${cacheRoot}`);
  }
  return { cacheRoot: resolvedCacheRoot, exists: true };
}

function inspectBucketLeases(bucketPath, hash) {
  const activeDir = path.join(bucketPath, ".active");
  let activeMetadata;
  try {
    activeMetadata = lstatSync(activeDir);
  } catch (error) {
    if (error.code === "ENOENT") return { liveLeases: [], unsafeReasons: [] };
    return {
      liveLeases: [],
      unsafeReasons: [`${hash}: cannot inspect lease directory: ${error.message}`],
    };
  }
  if (!activeMetadata.isDirectory() || activeMetadata.isSymbolicLink()) {
    return {
      liveLeases: [],
      unsafeReasons: [`${hash}: lease path is not a real directory`],
    };
  }

  let entries;
  try {
    entries = readdirSync(activeDir, { withFileTypes: true });
  } catch (error) {
    return {
      liveLeases: [],
      unsafeReasons: [`${hash}: cannot enumerate leases: ${error.message}`],
    };
  }

  const liveLeases = [];
  const unsafeReasons = [];
  for (const entry of entries) {
    const leasePath = path.join(activeDir, entry.name);
    if (!entry.isFile() || !entry.name.endsWith(".json")) {
      unsafeReasons.push(`${hash}: unexpected or temporary lease ${entry.name}`);
      continue;
    }

    try {
      const metadata = lstatSync(leasePath);
      if (!metadata.isFile() || metadata.isSymbolicLink()) {
        throw new Error("lease is not a regular file");
      }
      const lease = JSON.parse(readFileSync(leasePath, "utf8"));
      if (
        !lease ||
        typeof lease !== "object" ||
        lease.version !== 1 ||
        !new Set(["pending", "running", "orphaned"]).has(lease.state)
      ) {
        throw new Error("lease schema is malformed");
      }
      liveLeases.push({ ...lease, leasePath });
    } catch (error) {
      unsafeReasons.push(`${hash}: malformed lease ${entry.name}: ${error.message}`);
    }
  }
  return { liveLeases, unsafeReasons };
}

export function listCargoCacheBuckets(commonRoot) {
  const cache = safeCacheRoot(commonRoot);
  if (!cache.exists) return [];
  const { cacheRoot } = cache;
  let entries;
  try {
    entries = readdirSync(cacheRoot, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }

  const buckets = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    if (!entry.isDirectory() || !/^[a-f0-9]{64}$/.test(entry.name)) continue;
    const bucketPath = path.join(cacheRoot, entry.name);
    const metadata = lstatSync(bucketPath);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) continue;
    const leaseState = inspectBucketLeases(bucketPath, entry.name);
    buckets.push({
      hash: entry.name,
      path: bucketPath,
      ...leaseState,
    });
  }
  return buckets;
}

export function createPruneSnapshot(cwd = process.cwd()) {
  const references = collectReferencedCargoHashes(cwd);
  return {
    ...references,
    buckets: listCargoCacheBuckets(references.commonRoot),
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

function probeUnixProcessGroup(lease) {
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
  return probeUnixProcessGroup(lease);
}

export function probeBuildProcessTreeAfterOwnedChildExit(lease) {
  if (!lease || typeof lease !== "object") {
    return { state: "unknown", reason: "invalid-lease" };
  }

  // The wrapper has just received its own child's exit event. On Unix, the
  // process-group probe is therefore sufficient even when a sandbox prevents
  // us from reading the system boot identity. Descendants in the group still
  // keep the lease alive.
  if (process.platform !== "win32") return probeUnixProcessGroup(lease);

  // Windows cannot prove that descendants survived an exited root without a
  // reliable process snapshot, so retain the regular fail-closed behavior.
  return probeBuildProcessTree(lease);
}

function finalizeLease(leasePath, probeBuildProcessTreeFn) {
  let lease;
  try {
    lease = JSON.parse(readFileSync(leasePath, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return true;
    return false;
  }

  const probe = probeBuildProcessTreeFn(lease);
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

export function finalizeLeaseAfterTreeExit(leasePath) {
  return finalizeLease(leasePath, probeBuildProcessTree);
}

export function finalizeLeaseAfterOwnedChildExit(leasePath) {
  return finalizeLease(leasePath, probeBuildProcessTreeAfterOwnedChildExit);
}

function classifyBucketLeases(bucket, probeBuildProcessTreeFn) {
  const unsafeReasons = [...(bucket.unsafeReasons ?? [])];
  let keep = unsafeReasons.length > 0;

  for (const lease of bucket.liveLeases ?? []) {
    if (lease?.state === "pending") {
      keep = true;
      unsafeReasons.push(`${bucket.hash}: pending lease cannot be pruned`);
      continue;
    }
    if (
      lease?.state === "orphaned" &&
      lease.bootIdentity === currentBootIdentity()
    ) {
      keep = true;
      unsafeReasons.push(`${bucket.hash}: current-boot orphan lease is unsafe`);
      continue;
    }

    let probe;
    try {
      probe = probeBuildProcessTreeFn(lease);
    } catch (error) {
      probe = { state: "unknown", reason: `process-tree-probe-failed-${error.message}` };
    }
    if (probe?.state === "active") {
      keep = true;
    } else if (probe?.state !== "empty") {
      keep = true;
      unsafeReasons.push(
        `${bucket.hash}: ${probe?.reason ?? "lease-state-unknown"}`,
      );
    }
  }

  return { keep, unsafeReasons };
}

export function planPrune({
  referencedHashes,
  buckets,
  probeBuildProcessTree: probeBuildProcessTreeFn,
}) {
  const keep = [];
  const remove = [];
  const unsafeReasons = [];

  for (const bucket of buckets) {
    const leaseState = classifyBucketLeases(
      bucket,
      probeBuildProcessTreeFn,
    );
    unsafeReasons.push(...leaseState.unsafeReasons);
    (referencedHashes.has(bucket.hash) || leaseState.keep ? keep : remove).push(
      bucket,
    );
  }

  return { keep, remove, unsafeReasons };
}

function failedApply(deleted, error) {
  return {
    ok: false,
    deleted,
    error: error instanceof Error ? error.message : String(error),
  };
}

export function applyPrunePlan({
  cwd = process.cwd(),
  plannedBuckets,
  probeBuildProcessTree: probeBuildProcessTreeFn = probeBuildProcessTree,
  removeBucket = (bucket) =>
    rmSync(bucket.path, { recursive: true, force: false }),
}) {
  const deleted = [];
  const plannedHashes = [];
  try {
    for (const bucket of plannedBuckets) {
      if (!/^[a-f0-9]{64}$/.test(bucket?.hash ?? "")) {
        throw new Error(`refusing non-hash cache bucket: ${bucket?.hash ?? "<missing>"}`);
      }
      if (!plannedHashes.includes(bucket.hash)) plannedHashes.push(bucket.hash);
    }
    plannedHashes.sort();

    // Preflight every worktree and lockfile before the first deletion.
    createPruneSnapshot(cwd);
  } catch (error) {
    return failedApply(deleted, `apply preflight failed: ${error.message}`);
  }

  for (const hash of plannedHashes) {
    let snapshot;
    try {
      // Re-enumerate all worktrees and leases immediately before each deletion.
      snapshot = createPruneSnapshot(cwd);
    } catch (error) {
      return failedApply(deleted, `apply revalidation failed: ${error.message}`);
    }

    if (snapshot.referencedHashes.has(hash)) {
      return failedApply(deleted, `apply revalidation kept referenced bucket ${hash}`);
    }
    const bucket = snapshot.buckets.find((candidate) => candidate.hash === hash);
    if (!bucket) continue;
    const freshPlan = planPrune({
      referencedHashes: snapshot.referencedHashes,
      buckets: [bucket],
      probeBuildProcessTree: probeBuildProcessTreeFn,
    });
    if (freshPlan.remove.length !== 1) {
      const detail = freshPlan.unsafeReasons.join("; ") || "active lease";
      return failedApply(
        deleted,
        `apply revalidation kept unsafe bucket ${hash}: ${detail}`,
      );
    }

    try {
      const metadata = lstatSync(bucket.path);
      if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
        throw new Error("bucket is no longer a real directory");
      }
      removeBucket(bucket);
      deleted.push(hash);
    } catch (error) {
      return failedApply(deleted, `failed to delete ${hash}: ${error.message}`);
    }
  }

  return { ok: true, deleted, error: null };
}
