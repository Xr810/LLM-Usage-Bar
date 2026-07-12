import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";

const requiredKeys = [
  "common.retry",
  "common.name",
  "common.disabled",
  "common.enable",
  "common.disable",
  "common.visible",
  "common.hidden",
  "common.hide",
  "common.show",
  "dashboardModules.navigation",
  "dashboardModules.selectModule",
  "dashboardModules.more",
  "dashboardModules.noneVisible",
  "dashboardModules.manage",
  "dashboardModules.settingsTitle",
  "dashboardModules.settingsDescription",
  "dashboardModules.newName",
  "dashboardModules.create",
  "dashboardModules.apiType",
  "dashboardModules.subscriptionType",
  "dashboardModules.providerCount_one",
  "dashboardModules.providerCount_other",
  "dashboardModules.nameFor",
  "dashboardModules.saveNamed",
  "dashboardModules.moveUp",
  "dashboardModules.moveDown",
  "dashboardModules.hideNamed",
  "dashboardModules.showNamed",
  "dashboardModules.hide",
  "dashboardModules.show",
  "dashboardModules.deleteBlocked",
  "dashboardModules.deleteNamed",
  "dashboardModules.deleteTitle",
  "dashboardModules.deleteMessage",
  "dashboardModules.navigationImpact",
  "settings.usageConfigurationDescription",
  "settings.sections",
  "settings.usageModules",
  "settings.provider",
  "settings.proxyRouting",
  "preview.currentUi",
  "usageDashboard.inputTokens",
  "usageDashboard.outputTokens",
  "usageDashboard.cacheReadTokens",
  "usageDashboard.cacheCreationTokens",
  "usageDashboard.totalTokens",
  "usageDashboard.lastUpdated",
  "usageDashboard.resetsAt",
  "usageDashboard.resetsIn",
  "usageDashboard.durationDays",
  "usageDashboard.durationHours",
  "usageDashboard.durationMinutes",
  "usageDashboard.quotaWindowUnavailable",
  "usageDashboard.providersInModule",
  "usageDashboard.subscriptionModuleEmpty",
  "usageDashboard.addProviderInSettings",
  "usageDashboard.apiOverview",
  "usageDashboard.costComplete",
  "usageDashboard.costEstimated",
  "usageDashboard.costPartial",
  "usageDashboard.costUnavailable",
  "usageDashboard.costEstimatedSummary",
  "usageDashboard.costUnavailableSummary",
  "usageDashboard.aggregateUnavailable",
  "usageDashboard.apiModuleEmpty",
  "usageDashboard.subscriptionModuleRequired",
  "usageDashboard.usageModule",
  "usageDashboard.chooseSubscriptionModule",
  "usageDashboard.newModuleName",
  "usageDashboard.createModule",
  "usageDashboard.providersSettingsDescription",
  "usageDashboard.noProvidersConfigured",
  "usageDashboard.editProvider",
  "usageDashboard.disableProvider",
  "usageDashboard.enableProvider",
  "usageDashboard.localProxy",
  "usageDashboard.localProxyDescription",
  "usageDashboard.proxyRunning",
  "usageDashboard.proxyStopped",
  "usageDashboard.proxyForwardingTargets",
  "usageDashboard.proxyForwardingDescription",
  "usageDashboard.leaveModuleEmptyTitle",
  "usageDashboard.leaveModuleEmptyMessage",
  "usageDashboard.moveProvider",
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

describe("dynamic dashboard locale copy", () => {
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

  it("uses localized preview copy outside English", () => {
    const english = readKey(en, "preview.currentUi");
    expect(readKey(zh, "preview.currentUi")).not.toBe(english);
    expect(readKey(zhTW, "preview.currentUi")).not.toBe(english);
    expect(readKey(ja, "preview.currentUi")).not.toBe(english);
  });
});
