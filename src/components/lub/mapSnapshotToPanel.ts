import type {
  TrayProviderUsageView,
  TrayUsageSnapshot,
} from "@/types/trayUsage";
import { providerDisplayName } from "@/components/tray-usage/trayUsagePresentation";
import { parseNonNegative } from "./capMath";
import {
  DEFAULT_ALERT_THRESHOLD_PCT,
  type LubPanelModel,
  type LubSourceModel,
  type LubSubtitleKey,
} from "./panelModel";

function monogram(name: string): string {
  const letter = name.trim().charAt(0);
  return letter ? letter.toUpperCase() : "?";
}

function subtitleKey(provider: TrayProviderUsageView): LubSubtitleKey {
  if (provider.billingKind === "subscription") {
    const preset =
      `${provider.systemPresetKey ?? ""} ${provider.providerId}`.toLowerCase();
    if (preset.includes("codex") || preset.includes("chatgpt")) {
      return "plan_codex";
    }
    return "plan_hour_window";
  }
  return "api_key";
}

function primaryUsedPercent(provider: TrayProviderUsageView): number | null {
  const windows = provider.subscription?.windows ?? [];
  // C4: an empty window list publishes usedPct=0 only as a compatibility
  // value. That 0 is not a measurement and must not be drawn.
  if (windows.length === 0) return null;
  const fiveHour = windows.find((window) => window.kind === "five_hour");
  const chosen = fiveHour ?? windows[0];
  if (chosen.usedPercent == null || chosen.usedPercent.trim() === "") {
    return null;
  }
  return parseNonNegative(chosen.usedPercent);
}

function panelState(
  budget: TrayUsageSnapshot["apiBudget"],
  alertThresholdPct: number,
): LubPanelModel["state"] {
  const consumed = parseNonNegative(budget.budgetConsumedPercent);
  if (budget.level != null) {
    // C10 / C12: warn follows level. Pace status stays on the dot only.
    return budget.level === "ok" ? "ok" : "warn";
  }
  // TODO(BE-20): drop this once `level` is always on the snapshot.
  const capped = budget.status === "red" && consumed != null && consumed >= 100;
  const warn = !capped && consumed != null && consumed >= alertThresholdPct;
  return capped ? "capped" : warn ? "warn" : "ok";
}

function sourceFromProvider(
  provider: TrayProviderUsageView,
  sharedCapUsd: string | null,
  onOpenSettings: (providerId: string | null) => void,
  readFailed: boolean,
): LubSourceModel {
  const name = providerDisplayName(
    provider.providerName,
    provider.subscription?.planLabel,
  );
  if (provider.billingKind === "subscription") {
    const usedPercent = primaryUsedPercent(provider);
    return {
      id: provider.providerId,
      name,
      monogram: monogram(provider.providerName),
      subtitleKey: subtitleKey(provider),
      enforceable: false,
      amountUsd: null,
      usedPercent,
      scaleCapUsd: null,
      costQuality: provider.recentUsage.costQuality,
      // C13 omits usedPct when the provider has no quota window.
      quotaGap: usedPercent == null ? (readFailed ? "error" : "stale") : null,
    };
  }

  const usage = provider.metered;
  const providerCap = usage?.dailyBudgetUsd ?? null;
  const scaleCap = sharedCapUsd ?? providerCap;
  const missingBudget = parseNonNegative(scaleCap) == null;
  return {
    id: provider.providerId,
    name,
    monogram: monogram(name),
    subtitleKey: "api_key",
    enforceable: true,
    amountUsd:
      usage?.costQuality === "unavailable"
        ? null
        : (usage?.todayCostUsd ?? null),
    usedPercent: null,
    scaleCapUsd: missingBudget ? null : scaleCap,
    costQuality: usage?.costQuality ?? "unavailable",
    onSetBudget: missingBudget
      ? () => onOpenSettings(provider.providerId)
      : undefined,
  };
}

/**
 * Maps the existing tray snapshot into the v5 panel.
 * Month spend, sessions, and the 14-day spark are not on this command.
 * Those stay null instead of being invented. No new Tauri command is called.
 *
 * `quota-updated` and `guard-state-changed` are emitted only after a real
 * change. Calling get_quota_status or get_guard_state does not emit them,
 * so this mapping never waits on those getters for a refresh.
 */
export function mapSnapshotToPanel(
  snapshot: TrayUsageSnapshot,
  onOpenSettings: (providerId: string | null) => void,
  alertThresholdPct: number = DEFAULT_ALERT_THRESHOLD_PCT,
): LubPanelModel {
  const budget = snapshot.apiBudget;
  const sharedCap = budget.mode === "shared" ? budget.dailyBudgetUsd : null;
  const showDayLine = parseNonNegative(sharedCap) != null;
  const readFailed = snapshot.refreshError != null;

  const sources = snapshot.agents.flatMap((agent) =>
    agent.providers
      .filter(
        (provider) =>
          (provider.billingKind === "subscription" &&
            provider.subscription != null) ||
          (provider.billingKind === "metered" && provider.metered != null),
      )
      .map((provider) =>
        sourceFromProvider(provider, sharedCap, onOpenSettings, readFailed),
      ),
  );

  return {
    state: panelState(budget, alertThresholdPct),
    todayUsd: budget.todayCostUsd,
    todayQuality: budget.costQuality,
    dailyCapUsd: budget.dailyBudgetUsd,
    showDayLine,
    monthUsd: null,
    monthQuality: "unavailable",
    monthlyCapUsd: null,
    showMonthLine: false,
    spark: null,
    sources,
    sessions: null,
    invalidCostRows: 0,
    sample: false,
    stale: snapshot.stale,
    betaOn: false,
    betaPercent: null,
    alertThresholdPct,
    onSetSharedBudget:
      budget.mode === "shared" && !showDayLine
        ? () => onOpenSettings(null)
        : undefined,
  };
}
