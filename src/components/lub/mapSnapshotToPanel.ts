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
  const fiveHour = windows.find((window) => window.kind === "five_hour");
  const chosen = fiveHour ?? windows[0];
  return parseNonNegative(chosen?.usedPercent ?? null);
}

function sourceFromProvider(
  provider: TrayProviderUsageView,
  sharedCapUsd: string | null,
  onOpenSettings: (providerId: string | null) => void,
): LubSourceModel {
  const name = providerDisplayName(
    provider.providerName,
    provider.subscription?.planLabel,
  );
  if (provider.billingKind === "subscription") {
    return {
      id: provider.providerId,
      name,
      monogram: monogram(provider.providerName),
      subtitleKey: subtitleKey(provider),
      enforceable: false,
      amountUsd: null,
      usedPercent: primaryUsedPercent(provider),
      scaleCapUsd: null,
      costQuality: provider.recentUsage.costQuality,
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
 */
export function mapSnapshotToPanel(
  snapshot: TrayUsageSnapshot,
  onOpenSettings: (providerId: string | null) => void,
  alertThresholdPct: number = DEFAULT_ALERT_THRESHOLD_PCT,
): LubPanelModel {
  const budget = snapshot.apiBudget;
  const consumed = parseNonNegative(budget.budgetConsumedPercent);
  const sharedCap = budget.mode === "shared" ? budget.dailyBudgetUsd : null;
  const showDayLine = parseNonNegative(sharedCap) != null;
  const capped = budget.status === "red" && consumed != null && consumed >= 100;
  const warn = !capped && consumed != null && consumed >= alertThresholdPct;

  const sources = snapshot.agents.flatMap((agent) =>
    agent.providers
      .filter(
        (provider) =>
          (provider.billingKind === "subscription" &&
            provider.subscription != null) ||
          (provider.billingKind === "metered" && provider.metered != null),
      )
      .map((provider) =>
        sourceFromProvider(provider, sharedCap, onOpenSettings),
      ),
  );

  return {
    state: capped ? "capped" : warn ? "warn" : "ok",
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
