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
  "common.unavailable",
  "dashboardAgents.navigation",
  "dashboardAgents.selectAgent",
  "dashboardAgents.more",
  "dashboardAgents.noneVisible",
  "dashboardAgents.manage",
  "dashboardAgents.settingsTitle",
  "dashboardAgents.settingsDescription",
  "dashboardAgents.customName",
  "dashboardAgents.createCustom",
  "dashboardAgents.fixed",
  "dashboardAgents.custom",
  "dashboardAgents.providerCount_one",
  "dashboardAgents.providerCount_other",
  "dashboardAgents.nameFor",
  "dashboardAgents.saveNamed",
  "dashboardAgents.moveUp",
  "dashboardAgents.moveDown",
  "dashboardAgents.hideNamed",
  "dashboardAgents.showNamed",
  "dashboardAgents.deleteNamed",
  "dashboardAgents.navigationImpact",
  "dashboardAgents.providerBindings",
  "dashboardAgents.addProviderFor",
  "dashboardAgents.chooseProvider",
  "dashboardAgents.addBindingFor",
  "dashboardAgents.addBinding",
  "dashboardAgents.requestedState",
  "dashboardAgents.effectiveState",
  "dashboardAgents.noApiKeyRequired",
  "dashboardAgents.credentialConfigured",
  "dashboardAgents.credentialMissing",
  "dashboardAgents.credentialUnavailable",
  "dashboardAgents.setApiKey",
  "dashboardAgents.replaceApiKey",
  "dashboardAgents.clearApiKey",
  "dashboardAgents.apiKey",
  "dashboardAgents.keyDescription",
  "dashboardAgents.clearKeyDescription",
  "dashboardAgents.enableProvider",
  "dashboardAgents.disableProvider",
  "dashboardAgents.deleteBinding",
  "agentProxySetup.agent",
  "agentProxySetup.namedTitle",
  "agentProxySetup.readOnly",
  "agentProxySetup.proxyOrigin",
  "agentProxySetup.routes",
  "agentProxySetup.protocol",
  "agentProxySetup.localBaseUrl",
  "agentProxySetup.credentialPlacement",
  "agentProxySetup.credentialStatus",
  "agentProxySetup.noRoutes",
  "agentProxySetup.claudeDesktopCredential",
  "usageDiagnostics.title",
  "usageDiagnostics.description",
  "usageDiagnostics.unassignedCount_one",
  "usageDiagnostics.unassignedCount_other",
  "usageDiagnostics.unassignedGroups",
  "usageDiagnostics.archivedHistory",
  "usageDiagnostics.invalidLinks",
  "usageDiagnostics.invalidLinkCount_one",
  "usageDiagnostics.invalidLinkCount_other",
  "settings.usageConfigurationDescription",
  "settings.sections",
  "settings.agents",
  "settings.providers",
  "settings.proxySetup",
  "settings.diagnostics",
  "preview.currentUi",
  "usageDashboard.agentEmpty",
  "usageDashboard.configureProviders",
  "usageDashboard.subscriptionAccounts",
  "usageDashboard.meteredUsage",
  "usageDashboard.meteredOverview",
  "usageDashboard.noSubscriptionHistory",
  "usageDashboard.noMeteredHistory",
  "usageDashboard.sharedAccount",
  "usageDashboard.sharedAccountQuota",
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
  "usageDashboard.costComplete",
  "usageDashboard.costEstimated",
  "usageDashboard.costPartial",
  "usageDashboard.costUnavailable",
  "usageDashboard.costEstimatedSummary",
  "usageDashboard.costUnavailableSummary",
  "usageDashboard.providersSettingsDescription",
  "usageDashboard.noProvidersConfigured",
  "usageDashboard.editProvider",
  "usageDashboard.disableProvider",
  "usageDashboard.enableProvider",
  "usageDashboard.localProxy",
  "usageDashboard.localProxyDescription",
  "usageDashboard.proxyRunning",
  "usageDashboard.proxyStopped",
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
    ]) {
      const english = readKey(en, key);
      expect(readKey(zh, key), key).not.toBe(english);
      expect(readKey(zhTW, key), key).not.toBe(english);
      expect(readKey(ja, key), key).not.toBe(english);
    }
  });
});
