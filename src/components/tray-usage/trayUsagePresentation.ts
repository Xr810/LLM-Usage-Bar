import type {
  TrayCostQuality,
  TrayProviderUsageView,
  TrayUsageStatus,
} from "@/types/trayUsage";
import { inferIconForPreset } from "@/config/iconInference";

export type TrayUsageTranslate = (
  key: string,
  options?: Record<string, unknown> & { defaultValue?: string },
) => unknown;

export interface TrayProviderRow {
  agentModuleId: string;
  agentName: string;
  provider: TrayProviderUsageView;
}

const PLACEHOLDER = "—";
const STANDARD_USD_FRACTION_DIGITS = 2;
const MICRO_USD_FRACTION_DIGITS = 6;
const STATUS_PERCENT_BOUNDARIES = [20, 50, 80] as const;
const PERCENT_FRACTION_DIGIT_STEPS = [2, 3, 4, 6, 8, 12, 15] as const;

function parseNonNegativeNumber(value: string | null): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

/**
 * The Provider as a person names it: "ChatGPT Pro", not "ChatGPT" with the tier
 * filed underneath. The stored name is deliberately just the product — the tier
 * comes from the credential and only exists for accounts whose token reports
 * one, so it is appended here rather than baked into the name.
 */
export function providerDisplayName(
  name: string,
  planLabel?: string | null,
): string {
  const plan = planLabel?.trim();
  return plan ? `${name} ${plan}` : name;
}

export function formatUsd(value: string | null, locale: string): string {
  const parsed = parseNonNegativeNumber(value);
  if (parsed == null) return PLACEHOLDER;

  const maximumFractionDigits =
    parsed > 0 && parsed < 0.01
      ? MICRO_USD_FRACTION_DIGITS
      : STANDARD_USD_FRACTION_DIGITS;

  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    currencyDisplay: "narrowSymbol",
    minimumFractionDigits: STANDARD_USD_FRACTION_DIGITS,
    maximumFractionDigits,
  }).format(parsed);
}

export function formatPercent(value: string | null): string {
  const parsed = parseNonNegativeNumber(value);
  if (parsed == null) return PLACEHOLDER;

  for (const maximumFractionDigits of PERCENT_FRACTION_DIGIT_STEPS) {
    const formatted = new Intl.NumberFormat("en-US", {
      maximumFractionDigits,
    }).format(parsed);
    const displayed = Number(formatted.replace(/,/g, ""));
    const preservesStatusBoundary = STATUS_PERCENT_BOUNDARIES.every(
      (boundary) =>
        parsed === boundary ||
        (parsed < boundary && displayed < boundary) ||
        (parsed > boundary && displayed > boundary),
    );

    if (preservesStatusBoundary) return `${formatted}%`;
  }

  return `${parsed.toString()}%`;
}

export function clampPercentForProgress(value: string | null): number {
  const parsed = parseNonNegativeNumber(value);
  return parsed == null ? 0 : Math.min(100, parsed);
}

export function hasUsablePercent(value: string | null): boolean {
  return parseNonNegativeNumber(value) != null;
}

export function hasUsableUsd(value: string | null): boolean {
  return parseNonNegativeNumber(value) != null;
}

export function hasUsableBudget(value: string | null): boolean {
  const parsed = parseNonNegativeNumber(value);
  return parsed != null && parsed > 0;
}

export function formatCount(value: number, locale: string): string {
  if (!Number.isSafeInteger(value) || value < 0) return PLACEHOLDER;
  return new Intl.NumberFormat(locale).format(value);
}

export function formatTokenCount(value: number, _locale: string): string {
  if (!Number.isSafeInteger(value) || value < 0) return PLACEHOLDER;
  return new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

/**
 * `unreported` separates "this source never carries a reset time" from "the
 * window resets soon". Both used to collapse into the `—` placeholder, which
 * then read as a sentence: "Resets —". Naming the state lets the caller pick
 * copy that explains itself instead.
 */
export function formatResetTime(
  value: string | null,
  now: Date,
  locale: string,
  pendingText = "Pending refresh",
): { text: string; pending: boolean; unreported: boolean } {
  if (value == null) {
    return { text: PLACEHOLDER, pending: false, unreported: true };
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return { text: PLACEHOLDER, pending: false, unreported: true };
  }

  const remainingMs = timestamp - now.getTime();
  if (remainingMs <= 0) {
    return { text: pendingText, pending: true, unreported: false };
  }

  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "always" });

  if (remainingMs >= day) {
    return {
      text: formatter.format(Math.ceil(remainingMs / day), "day"),
      pending: false,
      unreported: false,
    };
  }
  if (remainingMs >= hour) {
    return {
      text: formatter.format(Math.ceil(remainingMs / hour), "hour"),
      pending: false,
      unreported: false,
    };
  }
  return {
    text: formatter.format(
      Math.max(1, Math.ceil(remainingMs / minute)),
      "minute",
    ),
    pending: false,
    unreported: false,
  };
}

export function formatUpdatedTime(
  value: number | null,
  now: Date,
  locale: string,
): string | null {
  if (value == null || !Number.isSafeInteger(value) || value < 0) return null;

  const differenceSeconds = value - now.getTime() / 1_000;
  const absoluteSeconds = Math.abs(differenceSeconds);
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "always" });

  if (absoluteSeconds >= 86_400) {
    return formatter.format(Math.round(differenceSeconds / 86_400), "day");
  }
  if (absoluteSeconds >= 3_600) {
    return formatter.format(Math.round(differenceSeconds / 3_600), "hour");
  }
  if (absoluteSeconds >= 60) {
    return formatter.format(Math.round(differenceSeconds / 60), "minute");
  }
  return formatter.format(Math.round(differenceSeconds), "second");
}

export function statusLabel(
  status: TrayUsageStatus,
  t: TrayUsageTranslate,
): string {
  const labels: Record<TrayUsageStatus, [string, string]> = {
    green: ["trayUsage.healthy", "Healthy"],
    yellow: ["trayUsage.warning", "Warning"],
    red: ["trayUsage.critical", "Critical"],
    unknown: ["trayUsage.unknown", "Data unavailable"],
  };
  const [key, defaultValue] = labels[status];
  return String(t(key, { defaultValue }));
}

export function costQualityLabel(
  quality: TrayCostQuality,
  t: TrayUsageTranslate,
): string {
  const labels: Record<TrayCostQuality, [string, string]> = {
    complete: ["trayUsage.completeData", "Complete"],
    estimated: ["trayUsage.estimatedData", "Estimated"],
    partial: ["trayUsage.partialData", "Partial data"],
    unavailable: ["trayUsage.unavailableData", "Unavailable"],
  };
  const [key, defaultValue] = labels[quality];
  return String(t(key, { defaultValue }));
}

const QUOTA_UNAVAILABLE_REASON_LABELS: Readonly<
  Record<string, [key: string, defaultValue: string]>
> = {
  invalid_quota_percent: [
    "trayUsage.invalidQuotaPercent",
    "Usage percentage unavailable",
  ],
  quota_unavailable: ["trayUsage.quotaUnavailable", "Quota data unavailable"],
  invalid_reset_timestamp: [
    "trayUsage.invalidResetTimestamp",
    "Reset time unavailable",
  ],
  reset_pending_refresh: [
    "trayUsage.resetPendingRefresh",
    "Waiting for refreshed quota",
  ],
};

export function quotaUnavailableReasonLabel(
  reason: string | null,
  t: TrayUsageTranslate,
): string | null {
  if (reason == null) return null;
  const label = QUOTA_UNAVAILABLE_REASON_LABELS[reason];
  if (!label) return null;
  const [key, defaultValue] = label;
  return String(t(key, { defaultValue }));
}

const PROVIDER_ICON_MAP: Readonly<Record<string, string>> = {
  "chatgpt-subscription": "openai",
  "claude-subscription": "claude",
  "openai-api": "openai",
  "anthropic-api": "anthropic",
  "openrouter-api": "openrouter",
};

export function providerIconName(preset: string | null): string | undefined {
  if (preset == null) return undefined;
  const mapped = PROVIDER_ICON_MAP[preset];
  if (mapped) return mapped;
  const { icon } = inferIconForPreset(preset);
  return icon;
}

/**
 * One-line explanation of why the rhythm profile moved this window's colour.
 *
 * Deliberately silent unless the rhythm actually changed the verdict: a note
 * on every row would be noise, and the point of this line is to answer the
 * one question the colour provokes — "why is 19% left green?". `flatStatus`
 * is what the same inputs would have produced without the profile, so a
 * difference is the precise definition of "the rhythm mattered here".
 */
export function rhythmExplanation(
  window: {
    status: TrayUsageStatus;
    flatStatus?: TrayUsageStatus | null;
    rhythmAdjustment?: string | null;
  },
  t: TrayUsageTranslate,
): string | null {
  const { status, flatStatus, rhythmAdjustment } = window;
  if (flatStatus == null || flatStatus === status) return null;

  const adjustment = Number(rhythmAdjustment);
  if (!Number.isFinite(adjustment) || adjustment <= 0) return null;
  if (adjustment === 1) return null;

  return adjustment < 1
    ? String(
        t("trayUsage.rhythmQuieter", {
          defaultValue:
            "You are usually quiet at this hour, so this should hold",
        }),
      )
    : String(
        t("trayUsage.rhythmBusier", {
          defaultValue: "You usually burn hard at this hour",
        }),
      );
}
