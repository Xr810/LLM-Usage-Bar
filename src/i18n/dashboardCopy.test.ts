import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";

const requiredKeys = [
  "common.name",
  "common.disabled",
  "common.enable",
  "common.disable",
  "dashboardAgents.settingsDescription",
  "agentProxySetup.readOnly",
  "usageDiagnostics.title",
  "usageDiagnostics.description",
  "usageDiagnostics.unassignedGroups",
  "usageDiagnostics.archivedHistory",
  "usageDiagnostics.invalidLinks",
  "settings.usageConfigurationDescription",
  "settings.sections",
  "settings.providers",
  "settings.diagnostics",
  "preview.currentUi",
  "usageDashboard.oneYear",
  "usageDashboard.configureProviders",
  "usageDashboard.subscriptionAccounts",
  "usageDashboard.remainingQuota",
  "usageDashboard.accountCount",
  "usageDashboard.selectedRangeTokens",
  "usageDashboard.selectedRangeTokensHint",
  "usageDashboard.dailyActivity",
  "usageDashboard.activityRange",
  "usageDashboard.activityChartLabel",
  "usageDashboard.activeDays",
  "usageDashboard.activityInteractionHint",
  "usageDashboard.recentStreak",
  "usageDashboard.lowActivity",
  "usageDashboard.highActivity",
  "usageDashboard.recordCount",
  "usageDashboard.records",
  "usageDashboard.recordsHint",
  "usageDashboard.peakTokens",
  "usageDashboard.meteredOverview",
  "usageDashboard.inputTokens",
  "usageDashboard.outputTokens",
  "usageDashboard.cacheReadTokens",
  "usageDashboard.cacheCreationTokens",
  "usageDashboard.totalTokens",
  "usageDashboard.totalTokensNamed",
  "usageDashboard.lastUpdated",
  "usageDashboard.resetsAt",
  "usageDashboard.resetsIn",
  "usageDashboard.durationDays",
  "usageDashboard.durationHours",
  "usageDashboard.durationMinutes",
  "usageDashboard.quotaWindowUnavailable",
  "usageDashboard.costComplete",
  "usageDashboard.costEstimated",
  "usageDashboard.costPartial",
  "usageDashboard.costUnavailable",
  "usageDashboard.costEstimatedSummary",
  "usageDashboard.costUnavailableSummary",
  "usageDashboard.providersSettingsDescription",
  "usageDashboard.customProviders",
  "usageDashboard.noCustomProvidersConfigured",
  "usageDashboard.fixedSystemProvider",
  "usageDashboard.apiKeyPlaceholder",
  "usageDashboard.verifyApiKey",
  "usageDashboard.dailyBudgetLabel",
  "usageDashboard.invalidDailyBudget",
  "usageDashboard.dailyBudgetUpdateFailed",
  "usageDashboard.saveDailyBudget",
  "usageDashboard.clearDailyBudget",
  "usageDashboard.apiSpendingLimits",
  "usageDashboard.apiSpendingLimitsDescription",
  "usageDashboard.independentProviderBudgets",
  "usageDashboard.independentProviderBudgetsDescription",
  "usageDashboard.sharedApiDailyBudget",
  "usageDashboard.sharedApiDailyBudgetDescription",
  "usageDashboard.perProviderBudgetsActiveDescription",
  "usageDashboard.apiBudgetUpdateFailed",
  "usageDashboard.officialPricingTitle",
  "usageDashboard.officialPricingDescription",
  "usageDashboard.officialPricingLastRefresh",
  "usageDashboard.officialPricingImportedModels",
  "usageDashboard.officialPricingRefreshNow",
  "usageDashboard.officialPricingRefreshing",
  "usageDashboard.officialPricingRefreshFailed",
  "usageDashboard.customPricingTitle",
  "usageDashboard.customPricingEmptyHint",
  "usageDashboard.customPricingCount",
  "usageDashboard.customPricingDescription",
  "usageDashboard.customPricingRowSummary",
  "usageDashboard.customPricingModelId",
  "usageDashboard.customPricingModelIdHint",
  "usageDashboard.customPricingInputLabel",
  "usageDashboard.customPricingOutputLabel",
  "usageDashboard.customPricingCacheReadLabel",
  "usageDashboard.customPricingCacheWriteLabel",
  "usageDashboard.addCustomPrice",
  "usageDashboard.saveCustomPrice",
  "usageDashboard.deleteCustomPrice",
  "usage.preset1y",
  "usage.customRangeHint",
  "settings.usageThresholds",
  "settings.usageThresholdsDescription",
  "settings.usageWarningThreshold",
  "settings.usageCriticalThreshold",
  "settings.usageThresholdsPreview",
  "settings.usageThresholdsSaveFailed",
  "settings.saveUsageThresholds",
  "trayUsage.updated",
  "trayUsage.neverUpdated",
  "trayUsage.stale",
  "trayUsage.refreshFailed",
  "trayUsage.healthy",
  "trayUsage.warning",
  "trayUsage.critical",
  "trayUsage.unknown",
  "trayUsage.subscription",
  "trayUsage.apiSpending",
  "trayUsage.remaining",
  "trayUsage.resets",
  "trayUsage.pendingRefresh",
  "trayUsage.invalidQuotaPercent",
  "trayUsage.quotaUnavailable",
  "trayUsage.invalidResetTimestamp",
  "trayUsage.resetPendingRefresh",
  "trayUsage.today",
  "trayUsage.ofBudget",
  "trayUsage.setDailyBudget",
  "trayUsage.setProviderBudget",
  "trayUsage.combinedApiSpending",
  "trayUsage.enabledApiProviderCount",
  "trayUsage.combinedDailyLimit",
  "trayUsage.combinedApiBudgetProgress",
  "trayUsage.setCombinedApiBudget",
  "trayUsage.completeData",
  "trayUsage.estimatedData",
  "trayUsage.partialData",
  "trayUsage.unavailableData",
  "trayUsage.openDetails",
  "trayUsage.openProviderDetails",
  "trayUsage.refresh",
  "trayUsage.refreshing",
  "trayUsage.settings",
  "trayUsage.quit",
  "trayUsage.empty",
  "trayUsage.loading",
  "trayUsage.fiveHourAllowance",
  "trayUsage.sevenDayAllowance",
  "trayUsage.dailyBudget",
  "trayUsage.providerAllowance",
  "trayUsage.providerAllowanceForAgent",
  "trayUsage.plan",
  "usageDashboard.claudeCliDetected",
  "usageDashboard.claudeCliDetectedSignedOut",
  "usageDashboard.claudeApiKey",
  "usageDashboard.claudeAccount",
  "usageDashboard.claudeAuthenticated",
  "usageDashboard.claudeQuotaSampledAt",
  "usageDashboard.claudeQuotaNoSample",
  "usageDashboard.editProvider",
  "usageDashboard.disableProvider",
  "usageDashboard.enableProvider",
] as const;

function readKey(locale: object, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>(
      (value, segment) =>
        value && typeof value === "object"
          ? (value as Record<string, unknown>)[segment]
          : undefined,
      locale,
    );
}

describe("Agent dashboard locale copy", () => {
  it.each([
    ["en", en],
    ["zh", zh],
    ["zh-TW", zhTW],
    ["ja", ja],
  ])("defines every production key in %s", (_language, locale) => {
    for (const key of requiredKeys) {
      expect(readKey(locale, key), key).toEqual(expect.any(String));
      expect(String(readKey(locale, key)).trim(), key).not.toBe("");
    }
  });

  it("localizes the Agent-specific copy outside English", () => {
    for (const key of [
      "dashboardAgents.settingsDescription",
      "agentProxySetup.readOnly",
      "usageDiagnostics.description",
      "trayUsage.refreshFailed",
      "trayUsage.setProviderBudget",
      "trayUsage.providerAllowanceForAgent",
      "trayUsage.invalidQuotaPercent",
      "usageDashboard.customPricingDescription",
      "usageDashboard.customPricingEmptyHint",
    ]) {
      const english = readKey(en, key);
      expect(readKey(zh, key), key).not.toBe(english);
      expect(readKey(zhTW, key), key).not.toBe(english);
      expect(readKey(ja, key), key).not.toBe(english);
    }
  });

  it("keeps the tray usage object shape identical in every locale", () => {
    const trayKeys = (locale: object) =>
      Object.keys(
        (locale as Record<string, Record<string, unknown>>).trayUsage ?? {},
      ).sort();
    const englishKeys = trayKeys(en);

    expect(trayKeys(zh)).toEqual(englishKeys);
    expect(trayKeys(zhTW)).toEqual(englishKeys);
    expect(trayKeys(ja)).toEqual(englishKeys);
  });

  it("does not expose renderer-owned quota credential copy", () => {
    for (const locale of [en, zh, zhTW, ja]) {
      expect(readKey(locale, "usageDashboard.quotaConfig")).toBeUndefined();
      expect(
        readKey(locale, "usageDashboard.invalidQuotaConfig"),
      ).toBeUndefined();
    }
  });
});
