import { describe, expect, it } from "vitest";
import type {
  DashboardModuleView,
  ProviderUsageView,
  UsageDashboardView,
  UsageProviderView,
} from "@/types/usageDashboard";
import { projectDashboardModule } from "./usageDashboardProjection";

const subscriptionModule: DashboardModuleView = {
  id: "module-subscription",
  name: "Arbitrary subscription label",
  kind: "subscription",
  sortOrder: 1,
  visible: true,
  isSystem: false,
  providerCount: 1,
};

const apiModule: DashboardModuleView = {
  id: "module-api",
  name: "Arbitrary API label",
  kind: "api",
  sortOrder: 2,
  visible: true,
  isSystem: true,
  providerCount: 2,
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
    quotaSource: billingKind === "subscription" ? "codex" : null,
    quotaIntervalSeconds: billingKind === "subscription" ? 300 : null,
    routeAppType: billingKind === "metered" ? "codex" : null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: billingKind === "metered" ? "https://example.com" : null,
    hasRouteCredentials: billingKind === "metered",
    dashboardModuleId:
      billingKind === "subscription" ? subscriptionModule.id : null,
    ...overrides,
  };
}

function usage(
  value: UsageProviderView,
  overrides: Partial<ProviderUsageView> = {},
): ProviderUsageView {
  return {
    provider: value,
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

function dashboard(
  groups: UsageDashboardView["productGroups"],
): UsageDashboardView {
  return { startAt: 1, endAt: 2, warnings: [], productGroups: groups };
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

describe("projectDashboardModule", () => {
  it("merges historical product groups by stable Provider ID and keeps newest quota state", () => {
    const oldProvider = provider("shared", "subscription", { updatedAt: 1 });
    const currentProvider = provider("shared", "subscription", {
      productGroupId: "renamed-product",
      updatedAt: 2,
    });
    const result = projectDashboardModule(
      subscriptionModule,
      dashboard([
        productGroup("old-product", [
          usage(oldProvider, {
            eventCount: 2,
            inputTokens: 10,
            outputTokens: 20,
            cacheReadTokens: 30,
            cacheCreationTokens: 40,
            quota: {
              snapshotId: "old",
              fetchedAt: 10,
              fiveHourUtilizationPercent: "20",
              fiveHourResetsAt: null,
              sevenDayUtilizationPercent: null,
              sevenDayResetsAt: null,
              manualResetsRemaining: null,
            },
            quotaFetchState: {
              providerId: "shared",
              lastAttemptAt: 10,
              lastSuccessAt: 10,
              lastError: null,
              stale: false,
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
            quota: {
              snapshotId: "new",
              fetchedAt: 20,
              fiveHourUtilizationPercent: "30",
              fiveHourResetsAt: null,
              sevenDayUtilizationPercent: "40",
              sevenDayResetsAt: null,
              manualResetsRemaining: 2,
            },
            quotaFetchState: {
              providerId: "shared",
              lastAttemptAt: 20,
              lastSuccessAt: 20,
              lastError: null,
              stale: false,
            },
          }),
        ]),
      ]),
    );

    expect(result.kind).toBe("subscription");
    expect(result.providers).toHaveLength(1);
    expect(result.providers[0]).toMatchObject({
      eventCount: 5,
      inputTokens: 11,
      outputTokens: 22,
      cacheReadTokens: 33,
      cacheCreationTokens: 44,
      quota: { snapshotId: "new" },
      quotaFetchState: { lastAttemptAt: 20 },
      provider: { id: "shared", productGroupId: "renamed-product" },
    });
  });

  it("uses module ID membership for subscriptions and never the display name", () => {
    const included = usage(provider("included", "subscription"));
    const wrongModule = usage(
      provider("wrong-module", "subscription", {
        name: subscriptionModule.name,
        dashboardModuleId: "different-stable-id",
      }),
    );
    const disabled = usage(
      provider("disabled", "subscription", { enabled: false }),
    );
    const result = projectDashboardModule(
      subscriptionModule,
      dashboard([productGroup("group", [included, wrongModule, disabled])]),
    );

    expect(result.providers.map((row) => row.provider.id)).toEqual([
      "included",
    ]);
  });

  it("sums metered decimal strings exactly and excludes subscription totals", () => {
    const first = usage(provider("api-a", "metered"), {
      totalCostUsd: "0.1",
      inputTokens: 10,
    });
    const second = usage(provider("api-b", "metered"), {
      totalCostUsd: "0.2",
      inputTokens: 20,
      costSourceCounts: { upstream: 0, estimated: 1, unavailable: 0 },
    });
    const subscription = usage(provider("sub", "subscription"), {
      inputTokens: 999,
      totalCostUsd: "999",
    });
    const result = projectDashboardModule(
      apiModule,
      dashboard([productGroup("group", [subscription], [first, second])]),
    );

    expect(result).toMatchObject({
      kind: "api",
      totalCostUsd: "0.3",
      totalTokens: 48,
      requestCount: 2,
      costStatus: "estimated",
    });
  });

  it("keeps explicit zero known and marks mixed missing costs as partial", () => {
    const knownZero = usage(provider("known-zero", "metered"), {
      totalCostUsd: "0",
    });
    const unknown = usage(provider("unknown", "metered"), {
      totalCostUsd: null,
      costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
    });
    const result = projectDashboardModule(
      apiModule,
      dashboard([productGroup("group", [], [knownZero, unknown])]),
    );

    expect(result).toMatchObject({
      totalCostUsd: "0",
      costStatus: "partial",
    });
  });

  it("reports unavailable when every metered cost is unknown", () => {
    const result = projectDashboardModule(
      apiModule,
      dashboard([
        productGroup(
          "group",
          [],
          [
            usage(provider("unknown", "metered"), {
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
            }),
          ],
        ),
      ]),
    );

    expect(result).toMatchObject({
      totalCostUsd: null,
      costStatus: "unavailable",
    });
  });
});
