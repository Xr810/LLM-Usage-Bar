/**
 * Shared presentation helpers for the Provider monitoring dashboard.
 *
 * Status thresholds mirror the tray popover contract:
 * - greater than 50% remaining  → healthy (success)
 * - 20%–50% remaining (inclusive) → warning
 * - below 20% remaining → critical (danger)
 */

export type UsageTone = "success" | "warning" | "danger" | "muted";

export function parsePercentValue(
  value: string | null | undefined,
): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function toneFromRemainingPercent(
  remaining: number | null,
): UsageTone {
  if (remaining == null) return "muted";
  if (remaining > 50) return "success";
  if (remaining >= 20) return "warning";
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
