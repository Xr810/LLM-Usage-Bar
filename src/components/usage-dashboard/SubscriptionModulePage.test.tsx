import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type {
  DashboardModuleView,
  ProviderUsageView,
  UsageProviderView,
} from "@/types/usageDashboard";
import { SubscriptionModulePage } from "./SubscriptionModulePage";

const module: DashboardModuleView = {
  id: "module-team",
  name: "Team plans",
  kind: "subscription",
  sortOrder: 1,
  visible: true,
  isSystem: false,
  providerCount: 2,
};

function provider(id: string, name: string): UsageProviderView {
  return {
    id,
    name,
    billingKind: "subscription",
    productGroupId: "immutable-product",
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
    dashboardModuleId: module.id,
  };
}

function usage(
  id: string,
  name: string,
  percent: string,
  inputTokens: number,
): ProviderUsageView {
  return {
    provider: provider(id, name),
    eventCount: 2,
    inputTokens,
    outputTokens: 20,
    cacheReadTokens: 30,
    cacheCreationTokens: 40,
    totalCostUsd: null,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: 0 },
    quota: {
      snapshotId: `quota-${id}`,
      fetchedAt: 1_700_000_000,
      fiveHourUtilizationPercent: null,
      fiveHourResetsAt: null,
      sevenDayUtilizationPercent: percent,
      sevenDayResetsAt: "2030-01-01T00:00:00Z",
      manualResetsRemaining: 2,
    },
    quotaFetchState: {
      providerId: id,
      lastAttemptAt: 1_700_000_000,
      lastSuccessAt: 1_700_000_000,
      lastError: null,
      stale: false,
    },
  };
}

const actions = {
  onRefreshQuota: vi.fn().mockResolvedValue(undefined),
  onSyncSessions: vi.fn().mockResolvedValue(undefined),
};

describe("SubscriptionModulePage", () => {
  it("renders one Provider directly with separate quota and token details", () => {
    render(
      <SubscriptionModulePage
        module={module}
        providers={[usage("personal", "Personal plan", "45", 10)]}
        {...actions}
      />,
    );

    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByText("Personal plan")).toBeInTheDocument();
    expect(
      screen.getByText("This subscription does not provide this quota window"),
    ).toBeInTheDocument();
    expect(screen.queryByText("0% used")).toBeNull();
    expect(screen.getByText("Input")).toBeInTheDocument();
    expect(screen.getByText("Cache creation")).toBeInTheDocument();
    expect(screen.queryByText("USD")).toBeNull();
  });

  it("switches account instances by stable ID without mixing quota or token values", async () => {
    const user = userEvent.setup();
    render(
      <SubscriptionModulePage
        module={module}
        providers={[
          usage("personal", "Personal plan", "25", 111),
          usage("work", "Work plan", "75", 222),
        ]}
        {...actions}
      />,
    );

    const personalTab = screen.getByRole("tab", { name: "Personal plan" });
    expect(
      screen.getByRole("tablist", { name: "Providers in Team plans" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tabpanel", { name: "Personal plan" }),
    ).toContainElement(screen.getByText("25% used"));
    expect(personalTab).toHaveAttribute(
      "aria-controls",
      screen.getByRole("tabpanel", { name: "Personal plan" }).id,
    );
    expect(screen.queryByText("75% used")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Work plan" }));
    expect(
      screen.getByRole("tabpanel", { name: "Work plan" }),
    ).toContainElement(screen.getByText("75% used"));
    expect(screen.queryByText("25% used")).toBeNull();
    expect(screen.getByText("222")).toBeInTheDocument();
    expect(screen.queryByText("111")).toBeNull();
  });

  it("falls back to the remaining Provider when the selected instance is removed", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <SubscriptionModulePage
        module={module}
        providers={[
          usage("personal", "Personal plan", "25", 111),
          usage("work", "Work plan", "75", 222),
        ]}
        {...actions}
      />,
    );
    await user.click(screen.getByRole("tab", { name: "Work plan" }));
    expect(screen.getByText("75% used")).toBeInTheDocument();

    rerender(
      <SubscriptionModulePage
        module={module}
        providers={[usage("personal", "Personal plan", "25", 111)]}
        {...actions}
      />,
    );

    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.queryByText("75% used")).toBeNull();
  });
});
