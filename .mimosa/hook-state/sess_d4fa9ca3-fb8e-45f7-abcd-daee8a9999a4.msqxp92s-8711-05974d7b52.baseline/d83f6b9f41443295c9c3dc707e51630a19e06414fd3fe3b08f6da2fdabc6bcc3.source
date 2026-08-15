import type { TrayUsageStatus } from "@/types/trayUsage";

/**
 * Shared presentation helpers for the Provider monitoring dashboard.
 *
 * Colour comes from the backend verdict via `toneFromStatus`: the status is
 * projected from the measured burn rate against the window's reset clock, so
 * it cannot be recomputed here — the frontend never sees the rate history.
 *
 * `toneFromRemainingPercent` is the fallback for the handful of surfaces that
 * have a remaining percentage but no backend status. It mirrors the backend's
 * own static fallback (used when a Provider reports no reset time):
 * - greater than 50% remaining  → healthy (success)
 * - 20%–50% remaining (inclusive) → warning
 * - below 20% remaining → critical (danger)
 */

export type UsageTone = "success" | "warning" | "danger" | "muted";

export interface RemainingThresholds {
  warning: number;
  critical: number;
}

export const DEFAULT_REMAINING_THRESHOLDS: RemainingThresholds = {
  warning: 50,
  critical: 20,
};

export function toneFromStatus(
  status: TrayUsageStatus | null | undefined,
): UsageTone {
  switch (status) {
    case "green":
      return "success";
    case "yellow":
      return "warning";
    case "red":
      return "danger";
    case "unknown":
    default:
      return "muted";
  }
}

export function parsePercentValue(
  value: string | null | undefined,
): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function toneFromRemainingPercent(
  remaining: number | null,
  thresholds: RemainingThresholds = DEFAULT_REMAINING_THRESHOLDS,
): UsageTone {
  if (remaining == null) return "muted";
  if (remaining > thresholds.warning) return "success";
  if (remaining >= thresholds.critical) return "warning";
  return "danger";
}

export function toneFromBudgetConsumed(consumed: number | null): UsageTone {
  if (consumed == null) return "muted";
  if (consumed >= 100) return "danger";
  if (consumed >= 80) return "warning";
  return "success";
}

const TOKEN_FORMATTER = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

export function formatTokensCompact(value: number): string {
  if (!Number.isSafeInteger(value) || value < 0) return "—";
  return TOKEN_FORMATTER.format(value);
}

import { inferIconForPreset } from "@/config/iconInference";
import { providerIconName } from "../tray-usage/trayUsagePresentation";

interface IconCandidateProvider {
  systemPresetKey: string | null;
  productGroupId: string;
  name: string;
}

/** Resolve a brand icon for a Provider: system preset map first, then
 *  fuzzy inference over preset key / product group / display name. */
export function dashboardProviderIcon(provider: IconCandidateProvider): {
  icon?: string;
  iconColor?: string;
} {
  const mapped = providerIconName(provider.systemPresetKey);
  if (mapped) return { icon: mapped };
  for (const candidate of [
    provider.systemPresetKey,
    provider.productGroupId,
    provider.name,
  ]) {
    if (!candidate) continue;
    const { icon, iconColor } = inferIconForPreset(candidate);
    if (icon) return { icon, iconColor };
  }
  return {};
}
