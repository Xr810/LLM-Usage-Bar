import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { SubscriptionProviderCard } from "./SubscriptionProviderCard";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (
      key: string,
      options?: { count?: number; value?: string; defaultValue?: string },
    ) => {
      if (key === "usageDashboard.durationDays") {
        return `${options?.count} 天`;
      }
      if (key === "usageDashboard.resetsAt") {
        return `重置时间：${options?.value}`;
      }
      if (key === "usageDashboard.resetsIn") {
        return `${options?.value} 后`;
      }
      return options?.defaultValue ?? key;
    },
    i18n: { language: "zh", resolvedLanguage: "zh" },
  }),
}));

function subscriptionUsage(): ProviderUsageView {
  return {
    provider: {
      id: "subscription",
      name: "Subscription",
      billingKind: "subscription",
      productGroupId: "codex",
      tokenSources: ["session_log"],
      sessionSourceBindings: ["codex"],
      quotaSource: "codex",
      quotaIntervalSeconds: 300,
      routeAppType: null,
      enabled: true,
      needsReview: false,
      createdAt: 1,
      updatedAt: 1,
      routeBaseUrl: null,
      hasRouteCredentials: false,
      dashboardModuleId: "module-codex",
    },
    eventCount: 1,
    inputTokens: 10,
    outputTokens: 5,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalCostUsd: null,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
    quota: {
      snapshotId: "quota",
      fetchedAt: Date.now() / 1000,
      fiveHourUtilizationPercent: "25",
      fiveHourResetsAt: "2026-07-14T01:00:00.000Z",
      sevenDayUtilizationPercent: null,
      sevenDayResetsAt: null,
      manualResetsRemaining: null,
    },
    quotaFetchState: null,
  };
}

describe("SubscriptionProviderCard localized reset countdown", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-13T00:00:00.000Z"));
  });

  afterEach(() => vi.useRealTimers());

  it("uses the localized day unit instead of a hard-coded d suffix", () => {
    render(
      <SubscriptionProviderCard
        usage={subscriptionUsage()}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(screen.getByText(/2 天 后/)).toBeInTheDocument();
    expect(screen.queryByText(/2d/)).toBeNull();
  });
});
