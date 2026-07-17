import { describe, expect, it } from "vitest";
import type {
  AgentModuleView,
  ProviderUsageView,
  ProviderMonitoringDashboardView,
  UsageDashboardView,
  UsageProviderView,
} from "@/types/usageDashboard";
import {
  projectAgentDashboard,
  projectProviderDashboard,
} from "./usageDashboardProjection";

const agent: AgentModuleView = {
  id: "codex",
  name: "Codex",
  sortOrder: 1,
  visible: true,
  isFixed: true,
  archivedAt: null,
  providerCount: 3,
};

function provider(
  id: string,
  billingKind: "subscription" | "metered",
  overrides: Partial<UsageProviderView> = {},
): UsageProviderView {
  return {
    id,
    name: `Provider ${id}`,
    billingKind,
    productGroupId: "current-product",
    tokenSources: billingKind === "metered" ? ["proxy"] : ["session_log"],
    sessionSourceBindings: [],
    bindings: [],
    quotaSource: billingKind === "subscription" ? "codex" : null,
    quotaIntervalSeconds: billingKind === "subscription" ? 300 : null,
    dailyBudgetUsd: billingKind === "metered" ? "10" : null,
    routeAppType: billingKind === "metered" ? "codex" : null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: billingKind === "metered" ? "https://example.com" : null,
    hasRouteCredentials: billingKind === "metered",
    systemPresetKey: null,
    systemAuthKind: null,
    canonicalEndpoint: null,
    compatibleAgentModuleIds: [],
    upstreamCredentialStatus: "not_required",
    upstreamCredentialVersion: 0,
    canClearUpstreamCredential: false,
    lastConnectionTestAt: null,
    lastConnectionTestStatus: null,
    ...overrides,
  };
}

function usage(
  value: UsageProviderView,
  overrides: Partial<ProviderUsageView> = {},
): ProviderUsageView {
  return {
    provider: value,
    sharedAccount: false,
    eventCount: 1,
    inputTokens: 10,
    outputTokens: 2,
    cacheReadTokens: 3,
    cacheCreationTokens: 4,
    totalCostUsd: value.billingKind === "metered" ? "0.1" : null,
    costSourceCounts: {
      upstream: value.billingKind === "metered" ? 1 : 0,
      estimated: 0,
      unavailable: value.billingKind === "metered" ? 0 : 1,
    },
    quota: null,
    quotaFetchState: null,
    ...overrides,
  };
}

function productGroup(
  productGroupId: string,
  subscriptionProviders: ProviderUsageView[] = [],
  meteredProviders: ProviderUsageView[] = [],
): UsageDashboardView["productGroups"][number] {
  return {
    productGroupId,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalCostUsd: null,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: 0 },
    tokenSources: [],
    subscriptionProviders,
    meteredProviders,
  };
}

function dashboard(
  groups: UsageDashboardView["productGroups"],
  agentModuleId = agent.id,
): UsageDashboardView {
  return {
    agentModuleId,
    startAt: 1,
    endAt: 2,
    warnings: [],
    productGroups: groups,
  };
}

describe("projectAgentDashboard", () => {
  it("merges historical Provider rows once, ORs shared-account, and attaches newest quota once", () => {
    const oldProvider = provider("shared", "subscription", {
      enabled: false,
      updatedAt: 1,
    });
    const currentProvider = provider("shared", "subscription", {
      productGroupId: "renamed-product",
      enabled: false,
      updatedAt: 2,
    });
    const result = projectAgentDashboard(
      agent,
      dashboard([
        productGroup("old-product", [
          usage(oldProvider, {
            eventCount: 2,
            inputTokens: 10,
            outputTokens: 20,
            cacheReadTokens: 30,
            cacheCreationTokens: 40,
            sharedAccount: false,
            quota: {
              snapshotId: "old",
              fetchedAt: 10,
              fiveHourUtilizationPercent: "20",
              fiveHourResetsAt: null,
              sevenDayUtilizationPercent: null,
              sevenDayResetsAt: null,
              manualResetsRemaining: null,
            },
          }),
        ]),
        productGroup("renamed-product", [
          usage(currentProvider, {
            eventCount: 3,
            inputTokens: 1,
            outputTokens: 2,
            cacheReadTokens: 3,
            cacheCreationTokens: 4,
            sharedAccount: true,
            quota: {
              snapshotId: "new",
              fetchedAt: 20,
              fiveHourUtilizationPercent: "30",
              fiveHourResetsAt: null,
              sevenDayUtilizationPercent: "40",
              sevenDayResetsAt: null,
              manualResetsRemaining: 2,
            },
          }),
        ]),
      ]),
    );

    expect(result?.subscriptionProviders).toHaveLength(1);
    expect(result?.subscriptionProviders[0]).toMatchObject({
      sharedAccount: true,
      eventCount: 5,
      inputTokens: 11,
      outputTokens: 22,
      cacheReadTokens: 33,
      cacheCreationTokens: 44,
      quota: { snapshotId: "new" },
      provider: {
        id: "shared",
        productGroupId: "renamed-product",
        enabled: false,
      },
    });
  });

  it("preserves historical, disabled, and currently unbound Provider cards", () => {
    const historical = usage(
      provider("historical", "subscription", {
        enabled: false,
        bindings: [],
      }),
    );
    const result = projectAgentDashboard(
      agent,
      dashboard([productGroup("history", [historical])]),
    );

    expect(result?.subscriptionProviders.map((row) => row.provider.id)).toEqual(
      ["historical"],
    );
  });

  it("shows subscription and metered Providers together but totals metered rows only", () => {
    const subscription = usage(provider("sub", "subscription"), {
      inputTokens: 999,
      totalCostUsd: "999",
    });
    const first = usage(provider("api-a", "metered"), {
      totalCostUsd: "0.1",
      inputTokens: 10,
    });
    const second = usage(provider("api-b", "metered"), {
      totalCostUsd: "0.2",
      inputTokens: 20,
      costSourceCounts: { upstream: 0, estimated: 1, unavailable: 0 },
    });
    const result = projectAgentDashboard(
      agent,
      dashboard([productGroup("group", [subscription], [first, second])]),
    );

    expect(result).toMatchObject({
      subscriptionProviders: [{ provider: { id: "sub" } }],
      meteredProviders: [
        { provider: { id: "api-a" } },
        { provider: { id: "api-b" } },
      ],
      meteredTotalCostUsd: "0.3",
      meteredTotalTokens: 48,
      meteredRequestCount: 2,
      meteredCostStatus: "estimated",
    });
  });

  it("returns no projection for a response owned by another Agent", () => {
    const result = projectAgentDashboard(
      agent,
      dashboard(
        [productGroup("group", [], [usage(provider("wrong", "metered"))])],
        "claude-code",
      ),
    );

    expect(result).toBeNull();
  });

  it("keeps known zero and marks mixed missing metered cost as partial", () => {
    const knownZero = usage(provider("known-zero", "metered"), {
      totalCostUsd: "0",
    });
    const unknown = usage(provider("unknown", "metered"), {
      totalCostUsd: null,
      costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
    });
    const result = projectAgentDashboard(
      agent,
      dashboard([productGroup("group", [], [knownZero, unknown])]),
    );

    expect(result).toMatchObject({
      meteredTotalCostUsd: "0",
      meteredCostStatus: "partial",
    });
  });
});

describe("projectProviderDashboard", () => {
  it("keeps Provider accounts separate and totals all metered accounts", () => {
    const first = usage(provider("openai-personal", "metered"), {
      eventCount: 2,
      inputTokens: 10,
      totalCostUsd: "0.1",
    });
    const second = usage(provider("openai-work", "metered"), {
      eventCount: 3,
      inputTokens: 20,
      totalCostUsd: "0.2",
      costSourceCounts: { upstream: 0, estimated: 1, unavailable: 0 },
    });
    const subscription = usage(provider("chatgpt-personal", "subscription"));
    const dashboard: ProviderMonitoringDashboardView = {
      startAt: 1,
      endAt: 2,
      warnings: [],
      providers: [first, second, subscription],
    };

    expect(projectProviderDashboard(dashboard)).toMatchObject({
      subscriptionProviders: [
        { provider: { id: "chatgpt-personal" } },
      ],
      meteredProviders: [
        { provider: { id: "openai-personal" } },
        { provider: { id: "openai-work" } },
      ],
      meteredRequestCount: 5,
      meteredTotalCostUsd: "0.3",
      meteredCostStatus: "estimated",
    });
  });
});
