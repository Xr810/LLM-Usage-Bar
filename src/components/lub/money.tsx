import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import type { TrayCostQuality } from "@/types/trayUsage";

const PLACEHOLDER = "—";

/** Whole-dollar caps in the prototype are `$10`, not `$10.00`. */
export function displayUsd(text: string): string {
  return text.replace(/\.00$/, "");
}

/**
 * Clock time for a known reset instant. Null when the timestamp is missing
 * or not a real date. Never substitutes local midnight.
 */
export function formatResetInstant(
  nextResetAt: string | null | undefined,
  locale: string,
): string | null {
  if (nextResetAt == null || nextResetAt.trim() === "") return null;
  const parsed = new Date(nextResetAt);
  if (Number.isNaN(parsed.getTime())) return null;
  return new Intl.DateTimeFormat(locale, {
    hour: "numeric",
    minute: "2-digit",
  }).format(parsed);
}

/**
 * One caption instant when every known row agrees. Rows with a null or
 * invalid `nextResetAt` are skipped. No known time, or two different
 * instants, returns null.
 */
export function sharedNextResetAt(
  rows: readonly { nextResetAt?: string | null }[],
): string | null {
  let chosen: string | null = null;
  let chosenMs: number | null = null;
  for (const row of rows) {
    const raw = row.nextResetAt;
    if (raw == null || raw.trim() === "") continue;
    const ms = Date.parse(raw);
    if (!Number.isFinite(ms)) continue;
    if (chosenMs == null) {
      chosen = raw;
      chosenMs = ms;
      continue;
    }
    if (ms !== chosenMs) return null;
  }
  return chosen;
}

export interface MoneyParts {
  approximate: boolean;
  text: string;
  symbol: string;
  number: string;
}

export function splitUsd(
  value: string | null,
  locale: string,
  quality: TrayCostQuality = "complete",
): MoneyParts {
  if (quality === "unavailable" || value == null || value.trim() === "") {
    return {
      approximate: false,
      text: PLACEHOLDER,
      symbol: "",
      number: PLACEHOLDER,
    };
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return {
      approximate: false,
      text: PLACEHOLDER,
      symbol: "",
      number: PLACEHOLDER,
    };
  }
  const text = formatUsd(value, locale);
  const parts = new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    currencyDisplay: "narrowSymbol",
    minimumFractionDigits: 2,
    maximumFractionDigits: parsed > 0 && parsed < 0.01 ? 6 : 2,
  }).formatToParts(parsed);
  const symbol = parts
    .filter((part) => part.type === "currency")
    .map((part) => part.value)
    .join("");
  const number = parts
    .filter((part) => part.type !== "currency" && part.type !== "literal")
    .map((part) => part.value)
    .join("");
  return {
    approximate: quality === "estimated" || quality === "partial",
    text,
    symbol,
    number,
  };
}
