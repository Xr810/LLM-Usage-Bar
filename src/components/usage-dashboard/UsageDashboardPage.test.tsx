import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  DashboardModuleView,
  UsageProviderView,
} from "@/types/usageDashboard";
import { UsageDashboardPage } from "./UsageDashboardPage";

const mocks = vi.hoisted(() => ({
  dashboard: vi.fn(),
  events: vi.fn(),
  refreshQuota: vi.fn(),
  syncSession: vi.fn(),
}));

const ui = vi.hoisted(() => ({
  language: "en",
  onUsageRecorded: undefined as undefined | (() => void),
  refreshPending: false,
  syncPending: false,
}));

const zh: Record<string, string> = {
  "usageDashboard.today": "今天",
  "usageDashboard.sevenDays": "7 天",
  "usageDashboard.thirtyDays": "30 天",
  "usageDashboard.customRange": "自定义范围",
  "usageDashboard.sourceSession": "会话日志",
  "usageDashboard.sourceProxy": "本地代理",
  "usageDashboard.tokens": "令牌",
  "usageDashboard.requests": "请求",
  "usageDashboard.recentRequests": "近期请求",
  "usageDashboard.costUnavailable": "费用不可用",
};

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      (ui.language === "zh" ? zh[key] : undefined) ??
      options?.defaultValue ??
      key,
    i18n: { resolvedLanguage: ui.language, language: ui.language },
  }),
}));

vi.mock("@/hooks/useUsageEventBridge", () => ({
  useUsageEventBridge: (onUsageRecorded?: () => void) => {
    ui.onUsageRecorded = onUsageRecorded;
  },
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageDashboard: (...args: unknown[]) => mocks.dashboard(...args),
  useUsageEvents: (...args: unknown[]) => mocks.events(...args),
  useRefreshProviderQuota: () => ({
    mutateAsync: mocks.refreshQuota,
    isPending: ui.refreshPending,
  }),
  useSyncProviderSessionUsage: () => ({
    mutateAsync: mocks.syncSession,
    isPending: ui.syncPending,
  }),
}));

vi.mock("@/components/usage/UsageDateRangePicker", () => ({
  UsageDateRangePicker: ({
    onApply,
    triggerLabel,
  }: {
    onApply: (value: unknown) => void;
    triggerLabel?: string;
  }) => (
    <button
      type="button"
      onClick={() =>
        onApply({ preset: "custom", customStartDate: 100, customEndDate: 200 })
      }
    >
      {triggerLabel ?? "Custom range"}
    </button>
  ),
}));

const subscriptionModule: DashboardModuleView = {
  id: "module-subscription",
  name: "Personal usage",
  kind: "subscription",
  sortOrder: 1,
  visible: true,
  isSystem: false,
  providerCount: 1,
};

const apiModule: DashboardModuleView = {
  id: "module-api",
  name: "Metered usage",
  kind: "api",
  sortOrder: 2,
  visible: true,
  isSystem: true,
  providerCount: 1,
};

const subscriptionProvider: UsageProviderView = {
  id: "sub",
  name: "Official Subscription",
  billingKind: "subscription",
  productGroupId: "claude",
  tokenSources: ["session_log"],
  sessionSourceBindings: ["claude"],
  quotaSource: "claude",
  quotaIntervalSeconds: 300,
  routeAppType: null,
  enabled: true,
  needsReview: false,
  createdAt: 1,
  updatedAt: 1,
  routeBaseUrl: null,
  hasRouteCredentials: false,
  dashboardModuleId: subscriptionModule.id,
};

const meteredProvider: UsageProviderView = {
  ...subscriptionProvider,
  id: "api",
  name: "Metered API",
  billingKind: "metered",
  tokenSources: ["proxy"],
  sessionSourceBindings: [],
  quotaSource: null,
  quotaIntervalSeconds: null,
  routeAppType: "claude",
  routeBaseUrl: "https://example.com",
  hasRouteCredentials: true,
  dashboardModuleId: null,
};

function dashboardData() {
  return {
    startAt: 10,
    endAt: 20,
    warnings: [] as string[],
    productGroups: [
      {
        productGroupId: "claude",
        inputTokens: 100,
        outputTokens: 20,
        cacheReadTokens: 5,
        cacheCreationTokens: 3,
        totalCostUsd: "1.25",
        costSourceCounts: { upstream: 1, estimated: 1, unavailable: 1 },
        tokenSources: ["proxy", "session_log"] as const,
        subscriptionProviders: [
          {
            provider: subscriptionProvider,
            eventCount: 1,
            inputTokens: 60,
            outputTokens: 10,
            cacheReadTokens: 5,
            cacheCreationTokens: 0,
            totalCostUsd: null,
            costSourceCounts: { upstream: 0, estimated: 0, unavailable: 0 },
            quota: {
              snapshotId: "q",
              fetchedAt: 15,
              fiveHourUtilizationPercent: "25",
              fiveHourResetsAt: "soon",
              sevenDayUtilizationPercent: "50",
              sevenDayResetsAt: "later",
              manualResetsRemaining: 2,
            },
            quotaFetchState: {
              providerId: "sub",
              lastAttemptAt: 18,
              lastSuccessAt: 15,
              lastError: "timeout",
              stale: true,
            },
          },
        ],
        meteredProviders: [
          {
            provider: meteredProvider,
            eventCount: 2,
            inputTokens: 40,
            outputTokens: 10,
            cacheReadTokens: 0,
            cacheCreationTokens: 3,
            totalCostUsd: "1.25",
            costSourceCounts: { upstream: 1, estimated: 0, unavailable: 1 },
            quota: null,
            quotaFetchState: null,
          },
        ],
      },
    ],
  };
}

describe("UsageDashboardPage", () => {
  beforeEach(() => {
    Object.values(mocks).forEach((mock) => mock.mockReset());
    ui.language = "en";
    ui.onUsageRecorded = undefined;
    ui.refreshPending = false;
    ui.syncPending = false;
    mocks.dashboard.mockReturnValue({
      data: dashboardData(),
      isLoading: false,
      error: null,
    });
    mocks.events.mockReturnValue({
      data: { items: [], total: 0, page: 1, pageSize: 5 },
      error: null,
    });
    mocks.refreshQuota.mockResolvedValue({});
    mocks.syncSession.mockResolvedValue({ warnings: [], errors: [] });
  });

  it("renders only the selected module's monitoring data and keeps configuration off home", () => {
    render(<UsageDashboardPage selectedModule={subscriptionModule} />);

    expect(screen.getByText("Official Subscription")).toBeInTheDocument();
    expect(screen.queryByText("Metered API")).toBeNull();
    for (const label of [
      "Add Provider",
      "Static routes",
      "Start proxy",
      "Stop proxy",
      "Enable",
      "Disable",
    ]) {
      expect(screen.queryByText(label)).toBeNull();
    }
  });

  it("renders the unified API module without subscription quota", () => {
    render(<UsageDashboardPage selectedModule={apiModule} />);

    expect(screen.getByText("Metered API")).toBeInTheDocument();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.getByTestId("api-total-tokens")).toHaveTextContent("53");
    expect(screen.getByTestId("api-request-count")).toHaveTextContent("2");
    expect(screen.queryByText("5 hour")).toBeNull();
  });

  it("supports exact today, 7d, 30d and fixed custom ranges", () => {
    vi.useFakeTimers();
    const nowMs = new Date("2026-07-12T12:34:56.789Z").getTime();
    vi.setSystemTime(nowMs);
    render(<UsageDashboardPage selectedModule={subscriptionModule} />);
    const endDate = Math.floor(nowMs / 1000) + 1;
    const localNow = new Date(nowMs);
    const todayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate(),
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(todayStart, endDate);

    fireEvent.click(screen.getByRole("button", { name: "7 days" }));
    const sevenDayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate() - 6,
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(sevenDayStart, endDate);

    fireEvent.click(screen.getByRole("button", { name: "30 days" }));
    const thirtyDayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate() - 29,
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(thirtyDayStart, endDate);

    fireEvent.click(screen.getByRole("button", { name: "Custom range" }));
    expect(mocks.dashboard).toHaveBeenLastCalledWith(100, 200);
    vi.useRealTimers();
  });

  it("advances a live range on usage events and periodic ticks", () => {
    vi.useFakeTimers();
    const initialMs = new Date("2026-07-12T12:34:56.100Z").getTime();
    vi.setSystemTime(initialMs);
    try {
      const { unmount } = render(
        <UsageDashboardPage selectedModule={subscriptionModule} />,
      );
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        expect.any(Number),
        Math.floor(initialMs / 1000) + 1,
      );

      vi.setSystemTime(initialMs + 2_000);
      act(() => ui.onUsageRecorded?.());
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        expect.any(Number),
        Math.floor((initialMs + 2_000) / 1000) + 1,
      );

      act(() => vi.advanceTimersByTime(30_000));
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        expect.any(Number),
        Math.floor((initialMs + 32_000) / 1000) + 1,
      );
      unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("surfaces dashboard, refresh and session errors without hiding other modules", async () => {
    const data = dashboardData();
    data.warnings = ["partial dashboard"];
    mocks.dashboard.mockReturnValue({
      data,
      isLoading: false,
      error: new Error("dashboard failed"),
    });
    mocks.refreshQuota.mockRejectedValueOnce(new Error("refresh failed"));
    mocks.syncSession.mockResolvedValueOnce({
      warnings: ["session warning"],
      errors: ["session error"],
    });
    render(<UsageDashboardPage selectedModule={subscriptionModule} />);

    expect(screen.getByText("partial dashboard")).toBeInTheDocument();
    expect(
      screen.getByRole("alert", { name: "dashboard failed" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
    expect(await screen.findByText("refresh failed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));
    expect(await screen.findByText("session warning")).toBeInTheDocument();
    expect(
      (await screen.findByText("session error")).closest("[role=alert]"),
    ).not.toBeNull();
  });

  it("disables only quota and session actions while their mutations are pending", () => {
    ui.refreshPending = true;
    ui.syncPending = true;
    render(<UsageDashboardPage selectedModule={subscriptionModule} />);

    expect(
      screen.getByRole("button", { name: "Refresh quota" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Sync sessions" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Today" })).toBeEnabled();
  });

  it("opens Settings from an empty subscription module", () => {
    const onOpenSettings = vi.fn();
    const data = dashboardData();
    data.productGroups[0].subscriptionProviders = [];
    mocks.dashboard.mockReturnValue({ data, isLoading: false, error: null });
    render(
      <UsageDashboardPage
        selectedModule={subscriptionModule}
        onOpenSettings={onOpenSettings}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Add Provider in Settings" }),
    );
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("localizes range, source and unavailable event cost labels", () => {
    ui.language = "zh";
    mocks.events.mockReturnValue({
      data: {
        items: [
          {
            eventId: "no-cost",
            model: "model",
            totalCostUsd: null,
            costSource: "unavailable",
          },
        ],
        total: 1,
        page: 1,
        pageSize: 5,
      },
      error: null,
    });
    render(<UsageDashboardPage selectedModule={apiModule} />);

    for (const text of [
      "今天",
      "7 天",
      "30 天",
      "自定义范围",
      "本地代理",
      "费用不可用",
    ]) {
      expect(screen.getAllByText(text).length).toBeGreaterThan(0);
    }
  });
});
