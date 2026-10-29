const seen = new Set<string>();

/** One debug line per distinct failure. A missing command must not spam the console. */
export function debugUpgradeNotice(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  if (seen.has(message)) return;
  seen.add(message);
  console.debug("[upgrade-notice]", message);
}

export function resetUpgradeNoticeDebug(): void {
  seen.clear();
}
