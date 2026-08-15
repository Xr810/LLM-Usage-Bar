/**
 * Presentation helpers shared by the "By model" and "By agent" breakdowns.
 *
 * Both dimensions render the same shape — a named row, its share of the
 * range total, and the tokens / requests / cost triple — so the labelling,
 * cost-quality and share maths live here instead of in each page.
 */

import type { CostSourceCounts } from "@/types/usageDashboard";
import type { MeteredCostStatus } from "./usageDashboardProjection";

export type BreakdownTranslate = (
  key: string,
  options?: Record<string, unknown> & { defaultValue?: string },
) => string;

/**
 * Display names for the built-in product groups. Anything else is a
 * user-created Provider, whose group id is shown as-is.
 */
const PRODUCT_GROUP_LABELS: Readonly<
  Record<string, [key: string, defaultValue: string]>
> = {
  "claude-subscription": [
    "usageBreakdown.productClaudeSubscription",
    "Claude Pro/Max",
  ],
  "chatgpt-subscription": [
    "usageBreakdown.productChatgptSubscription",
    "ChatGPT Plus/Pro",
  ],
  "anthropic-api": ["usageBreakdown.productAnthropicApi", "Anthropic API"],
  "openai-api": ["usageBreakdown.productOpenaiApi", "OpenAI API"],
  "openrouter-api": ["usageBreakdown.productOpenrouterApi", "OpenRouter"],
};

/**
 * Label a product group. Built-in groups get a translated plan name; a custom
 * group falls back to the accounts inside it, and finally to the raw id — the
 * id is never hidden behind an invented name.
 */
export function productGroupLabel(
  productGroupId: string,
  providerNames: string[],
  t: BreakdownTranslate,
): string {
  const known = PRODUCT_GROUP_LABELS[productGroupId];
  if (known) {
    const [key, defaultValue] = known;
    return t(key, { defaultValue });
  }
  const named = providerNames.filter((name) => name.trim() !== "");
  if (named.length === 1) return named[0];
  if (named.length > 1) return named.join(" · ");
  return productGroupId;
}

/** Percentage of the range total, clamped to [0, 100]. `0` when total is 0. */
export function sharePercent(part: number, total: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(total) || total <= 0) return 0;
  return Math.min(100, Math.max(0, (part / total) * 100));
}

/** One decimal below 10%, none above — keeps a long ranked list scannable. */
export function formatSharePercent(share: number): string {
  if (share <= 0) return "0%";
  if (share < 0.1) return "<0.1%";
  return `${share < 10 ? share.toFixed(1) : Math.round(share)}%`;
}

/**
 * Cost trust for a single row. Mirrors the Provider dashboard's rules:
 * a missing total is `unavailable`, any unpriced event downgrades a known
 * total to `partial`, and estimated pricing is called out separately.
 */
export function rowCostStatus(
  totalCostUsd: string | null,
  counts: CostSourceCounts,
): MeteredCostStatus {
  if (totalCostUsd == null) return "unavailable";
  if (counts.unavailable > 0) return "partial";
  if (counts.estimated > 0) return "estimated";
  return "complete";
}

export function costStatusLabel(
  status: MeteredCostStatus,
  t: BreakdownTranslate,
): string {
  const labels: Record<MeteredCostStatus, [string, string]> = {
    complete: ["usageDashboard.costComplete", "Trusted cost data"],
    estimated: [
      "usageDashboard.costEstimatedSummary",
      "Includes estimated cost",
    ],
    partial: ["usageDashboard.costPartial", "Partial cost data"],
    unavailable: ["usageDashboard.costUnavailableSummary", "Cost unavailable"],
  };
  const [key, defaultValue] = labels[status];
  return t(key, { defaultValue });
}

/** `$1.25`, or the localized "cost unavailable" text when nothing is priced. */
export function costText(
  totalCostUsd: string | null,
  t: BreakdownTranslate,
): string {
  if (totalCostUsd == null) {
    return t("usageDashboard.costUnavailableSummary", {
      defaultValue: "Cost unavailable",
    });
  }
  return `$${totalCostUsd}`;
}

/**
 * A model id is an identifier, not prose: it is rendered verbatim in a mono
 * face so two snapshots of the same family stay distinguishable. Only an
 * empty id is substituted.
 */
export function modelLabel(model: string, t: BreakdownTranslate): string {
  const trimmed = model.trim();
  if (trimmed === "" || trimmed === "unknown") {
    return t("usageBreakdown.unknownModel", { defaultValue: "Unknown model" });
  }
  return trimmed;
}

/** Agent display name, with a distinct label for the no-owner bucket. */
export function agentLabel(
  agentModuleId: string | null,
  agentName: string | null,
  t: BreakdownTranslate,
): string {
  if (agentName != null && agentName.trim() !== "") return agentName;
  if (agentModuleId == null) {
    return t("usageBreakdown.unassignedAgent", { defaultValue: "Unassigned" });
  }
  return agentModuleId;
}
