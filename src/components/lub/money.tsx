import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import type { TrayCostQuality } from "@/types/trayUsage";

const PLACEHOLDER = "—";

/** Whole-dollar caps in the prototype are `$10`, not `$10.00`. */
export function displayUsd(text: string): string {
  return text.replace(/\.00$/, "");
}

/** Daily caps reset at local midnight. The snapshot has no reset timestamp. */
export function formatLocalMidnight(locale: string, now = new Date()): string {
  const midnight = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate(),
    0,
    0,
    0,
    0,
  );
  return new Intl.DateTimeFormat(locale, {
    hour: "numeric",
    minute: "2-digit",
  }).format(midnight);
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
