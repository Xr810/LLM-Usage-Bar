/** Money arrives as a decimal string so the backend never rounds it. Small
    spends are the common case on a metered key, so sub-dollar amounts keep
    enough digits to stay distinguishable from zero. The locale is the app's
    language, not the OS's — otherwise a Chinese UI on an English system (or
    the reverse) formats its amounts the other way round. */
export function formatUsd(
  value: string | null | undefined,
  locale: string,
): string | null {
  if (value === null || value === undefined || value.trim() === "") return null;
  const amount = Number(value);
  if (!Number.isFinite(amount)) return null;
  const fractionDigits = amount !== 0 && Math.abs(amount) < 1 ? 4 : 2;
  return amount.toLocaleString(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: fractionDigits,
  });
}

/** How much of a capped key is spent, or null when it has no cap to measure
    against. An uncapped key has no meaningful bar — drawing an empty one would
    read as "nothing left". */
export function spentRatio(
  limitUsd: string | null | undefined,
  limitRemainingUsd: string | null | undefined,
): number | null {
  const limit = Number(limitUsd ?? "");
  const remaining = Number(limitRemainingUsd ?? "");
  if (!Number.isFinite(limit) || !Number.isFinite(remaining) || limit <= 0) {
    return null;
  }
  return Math.min(1, Math.max(0, 1 - remaining / limit));
}

export function spendBarClass(ratio: number): string {
  if (ratio >= 0.9) return "bg-destructive";
  if (ratio >= 0.75) return "bg-warning";
  return "bg-success";
}
