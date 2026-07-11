import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageDashboardPage } from "./UsageDashboardPage";

const mocks = vi.hoisted(() => ({
  dashboard: vi.fn(),
  events: vi.fn(),
  providers: vi.fn(),
  bindings: vi.fn(),
  saveProvider: vi.fn(),
  setEnabled: vi.fn(),
  setBinding: vi.fn(),
  refreshQuota: vi.fn(),
  syncSession: vi.fn(),
  isRunning: vi.fn(),
  startProxy: vi.fn(),
  stopProxy: vi.fn(),
}));

const ui = vi.hoisted(() => ({
  language: "en",
  onUsageRecorded: undefined as undefined | (() => void),
  pending: {
    saveProvider: false,
    setEnabled: false,
    setBinding: false,
    refreshQuota: false,
    syncSession: false,
    startProxy: false,
    stopProxy: false,
  },
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
  useUsageProviders: () => mocks.providers(),
  useRouteBindings: () => mocks.bindings(),
  useSaveUsageProvider: () => ({
    mutateAsync: mocks.saveProvider,
    isPending: ui.pending.saveProvider,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: mocks.setEnabled,
    isPending: ui.pending.setEnabled,
  }),
  useSetRouteBinding: () => ({
    mutateAsync: mocks.setBinding,
    isPending: ui.pending.setBinding,
  }),
  useRefreshProviderQuota: () => ({
    mutateAsync: mocks.refreshQuota,
    isPending: ui.pending.refreshQuota,
  }),
  useSyncProviderSessionUsage: () => ({
    mutateAsync: mocks.syncSession,
    isPending: ui.pending.syncSession,
  }),
}));

vi.mock("@/lib/query/proxy", () => ({
  useIsProxyRunning: () => mocks.isRunning(),
  useStartProxyServer: () => ({
    mutateAsync: mocks.startProxy,
    isPending: ui.pending.startProxy,
  }),
  useStopProxyServer: () => ({
    mutateAsync: mocks.stopProxy,
    isPending: ui.pending.stopProxy,
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

const subscriptionProvider = {
  id: "sub",
  name: "Official Subscription",
  billingKind: "subscription" as const,
  productGroupId: "claude",
  tokenSources: ["session_log" as const],
  quotaSource: "claude",
  quotaIntervalSeconds: 300,
  routeAppType: null,
  enabled: true,
  needsReview: false,
  createdAt: 1,
  updatedAt: 1,
  routeBaseUrl: null,
  hasRouteCredentials: false,
};
const meteredProvider = {
  ...subscriptionProvider,
  id: "api",
  name: "Metered API",
  billingKind: "metered" as const,
  tokenSources: ["proxy" as const],
  quotaSource: null,
  routeAppType: "claude",
  routeBaseUrl: "https://example.com",
  hasRouteCredentials: true,
};

describe("UsageDashboardPage", () => {
  beforeEach(() => {
    Object.values(mocks).forEach((mock) => mock.mockReset());
    ui.language = "en";
    ui.onUsageRecorded = undefined;
    Object.keys(ui.pending).forEach((key) => {
      ui.pending[key as keyof typeof ui.pending] = false;
    });
    mocks.providers.mockReturnValue({
      data: [subscriptionProvider, meteredProvider],
      isLoading: false,
    });
    mocks.bindings.mockReturnValue({
      data: [{ protocol: "claude", providerId: "api", updatedAt: 1 }],
    });
    mocks.events.mockReturnValue({
      data: { items: [], total: 0, page: 1, pageSize: 5 },
    });
    mocks.isRunning.mockReturnValue({ data: false });
    mocks.dashboard.mockReturnValue({
      data: {
        startAt: 10,
        endAt: 20,
        warnings: [],
        productGroups: [
          {
            productGroupId: "claude",
            inputTokens: 100,
            outputTokens: 20,
            cacheReadTokens: 5,
            cacheCreationTokens: 3,
            totalCostUsd: "1.25",
            costSourceCounts: { upstream: 1, estimated: 1, unavailable: 1 },
            tokenSources: ["proxy", "session_log"],
            subscriptionProviders: [
              {
                provider: subscriptionProvider,
                eventCount: 1,
                inputTokens: 60,
                outputTokens: 10,
                cacheReadTokens: 5,
                cacheCreationTokens: 0,
                totalCostUsd: null,
                costSourceCounts: { upstream: 0, estimated: 1, unavailable: 0 },
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
      },
      isLoading: false,
    });
    mocks.setBinding.mockResolvedValue({});
    mocks.refreshQuota.mockResolvedValue({});
    mocks.syncSession.mockResolvedValue({
      warnings: ["no usage source binding for claude"],
    });
    mocks.startProxy.mockResolvedValue({});
    mocks.stopProxy.mockResolvedValue({});
  });

  it("renders separated cards, source/cost labels and stale last-success quota", () => {
    render(<UsageDashboardPage />);
    expect(
      within(screen.getByTestId("subscription-provider-sub")).getByText(
        "Official Subscription",
      ),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("metered-provider-api")).getByText(
        "Metered API",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Session log")).toBeInTheDocument();
    expect(screen.getByText("Proxy")).toBeInTheDocument();
    expect(screen.getAllByText("Upstream").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Estimated").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    expect(screen.getByText("Stale: timeout")).toBeInTheDocument();
    expect(screen.getByText("25% used")).toBeInTheDocument();
  });

  it("supports exact today, 7d, 30d and fixed custom ranges", () => {
    vi.useFakeTimers();
    const nowMs = new Date("2026-07-12T12:34:56.789Z").getTime();
    vi.setSystemTime(nowMs);
    render(<UsageDashboardPage />);
    const endDate = Math.floor(nowMs / 1000) + 1;
    const localNow = new Date(nowMs);
    const todayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate(),
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      todayStart,
      endDate,
      undefined,
    );
    for (const label of ["Today", "7 days", "30 days", "Custom range"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "7 days" }));
    const sevenDayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate() - 6,
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      sevenDayStart,
      endDate,
      undefined,
    );
    fireEvent.click(screen.getByRole("button", { name: "30 days" }));
    const thirtyDayStart = Math.floor(
      new Date(
        localNow.getFullYear(),
        localNow.getMonth(),
        localNow.getDate() - 29,
      ).getTime() / 1000,
    );
    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      thirtyDayStart,
      endDate,
      undefined,
    );
    fireEvent.click(screen.getByRole("button", { name: "Custom range" }));
    expect(mocks.dashboard).toHaveBeenLastCalledWith(100, 200, undefined);
    vi.useRealTimers();
  });

  it("advances the live query range on usage events and periodic ticks", () => {
    vi.useFakeTimers();
    const clearIntervalSpy = vi.spyOn(globalThis, "clearInterval");
    const initialMs = new Date("2026-07-12T12:34:56.100Z").getTime();
    vi.setSystemTime(initialMs);
    const { unmount } = render(<UsageDashboardPage />);

    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      expect.any(Number),
      Math.floor(initialMs / 1000) + 1,
      undefined,
    );

    const eventMs = initialMs + 5_000;
    vi.setSystemTime(eventMs);
    act(() => ui.onUsageRecorded?.());
    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      expect.any(Number),
      Math.floor(eventMs / 1000) + 1,
      undefined,
    );

    const tickMs = eventMs + 30_000;
    act(() => {
      vi.advanceTimersByTime(30_000);
    });
    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      expect.any(Number),
      Math.floor(tickMs / 1000) + 1,
      undefined,
    );

    fireEvent.click(screen.getByRole("button", { name: "Custom range" }));
    expect(mocks.dashboard).toHaveBeenLastCalledWith(100, 200, undefined);
    act(() => {
      vi.advanceTimersByTime(30_000);
    });
    expect(mocks.dashboard).toHaveBeenLastCalledWith(100, 200, undefined);

    unmount();
    expect(clearIntervalSpy).toHaveBeenCalled();
    clearIntervalSpy.mockRestore();
    vi.useRealTimers();
  });

  it("preserves zero quota refresh and rejects invalid intervals", async () => {
    render(<UsageDashboardPage />);
    fireEvent.click(
      within(screen.getByTestId("provider-config-sub")).getByRole("button", {
        name: "Edit",
      }),
    );
    const interval = screen.getByLabelText("Refresh interval (seconds)");
    expect(interval).toHaveAttribute("min", "0");
    expect(
      screen.getByText(
        "0 disables refresh; otherwise use at least 60 seconds.",
      ),
    ).toBeInTheDocument();

    fireEvent.change(interval, { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ quotaIntervalSeconds: 0 }),
      ),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );

    mocks.saveProvider.mockClear();
    fireEvent.click(
      within(screen.getByTestId("provider-config-sub")).getByRole("button", {
        name: "Edit",
      }),
    );
    fireEvent.change(screen.getByLabelText("Refresh interval (seconds)"), {
      target: { value: "59" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Refresh interval must be 0 or at least 60 seconds.",
    );
    expect(mocks.saveProvider).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("Refresh interval (seconds)"), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Refresh interval must be 0 or at least 60 seconds.",
    );
    expect(mocks.saveProvider).not.toHaveBeenCalled();
  });

  it("sends a Base URL edit for an existing provider without credentials", async () => {
    const provider = {
      ...meteredProvider,
      id: "auth-free",
      routeBaseUrl: "https://old.example.com",
      hasRouteCredentials: false,
    };
    mocks.providers.mockReturnValue({ data: [provider], isLoading: false });
    render(<UsageDashboardPage />);
    fireEvent.click(
      within(screen.getByTestId("provider-config-auth-free")).getByRole(
        "button",
        { name: "Edit" },
      ),
    );
    const baseUrl = screen.getByLabelText("Base URL");
    expect(baseUrl).not.toBeDisabled();
    fireEvent.change(baseUrl, { target: { value: "https://new.example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "auth-free",
          routeConfig: { baseUrl: "https://new.example.com" },
        }),
      ),
    );
  });

  it("keeps stored route credentials omitted and requires re-entry for a Base URL edit", async () => {
    render(<UsageDashboardPage />);
    fireEvent.click(
      within(screen.getByTestId("provider-config-api")).getByRole("button", {
        name: "Edit",
      }),
    );

    const secret = screen.getByLabelText(
      "API key (leave blank to keep stored key)",
    );
    const baseUrl = screen.getByLabelText("Base URL");
    expect(secret).toHaveValue("");
    expect(baseUrl).toBeDisabled();

    fireEvent.change(secret, { target: { value: "replacement-key" } });
    expect(baseUrl).not.toBeDisabled();
    fireEvent.change(baseUrl, { target: { value: "https://new.example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "api",
          routeConfig: {
            baseUrl: "https://new.example.com",
            apiKey: "replacement-key",
          },
        }),
      ),
    );
  });

  it("omits routeConfig when an existing secret is left blank", async () => {
    render(<UsageDashboardPage />);
    fireEvent.click(
      within(screen.getByTestId("provider-config-api")).getByRole("button", {
        name: "Edit",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith(
        expect.objectContaining({ id: "api", routeConfig: undefined }),
      ),
    );
  });

  it("saves routes, controls proxy, refreshes quota and surfaces session warnings", async () => {
    render(<UsageDashboardPage />);
    fireEvent.change(screen.getByLabelText("Claude route"), {
      target: { value: "api" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save routes" }));
    fireEvent.click(screen.getByRole("button", { name: "Start proxy" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));

    await waitFor(() =>
      expect(mocks.setBinding).toHaveBeenCalledWith({
        protocol: "claude",
        providerId: "api",
      }),
    );
    expect(mocks.startProxy).toHaveBeenCalled();
    expect(mocks.refreshQuota).toHaveBeenCalledWith("sub");
    expect(mocks.syncSession).toHaveBeenCalledWith("sub");
    expect(
      await screen.findByText("no usage source binding for claude"),
    ).toBeInTheDocument();

    mocks.isRunning.mockReturnValue({ data: true });
    const { rerender } = render(<UsageDashboardPage />);
    rerender(<UsageDashboardPage />);
    fireEvent.click(screen.getAllByRole("button", { name: "Stop proxy" })[0]);
    expect(mocks.stopProxy).toHaveBeenCalled();
  });

  it("renders card sources from provider tokenSources and usage metrics for both billing kinds", () => {
    const subscriptionWithProxy = {
      ...subscriptionProvider,
      tokenSources: ["proxy" as const],
    };
    const meteredWithSessions = {
      ...meteredProvider,
      tokenSources: ["session_log" as const],
    };
    const data = mocks.dashboard.mock.results[0]?.value?.data;
    mocks.dashboard.mockReturnValue({
      data: {
        ...(data ?? {
          startAt: 10,
          endAt: 20,
          warnings: [],
          productGroups: [],
        }),
        productGroups: [
          {
            productGroupId: "mixed",
            inputTokens: 11,
            outputTokens: 7,
            cacheReadTokens: 3,
            cacheCreationTokens: 2,
            totalCostUsd: "3.50",
            costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
            tokenSources: ["proxy", "session_log"],
            subscriptionProviders: [
              {
                provider: subscriptionWithProxy,
                eventCount: 4,
                inputTokens: 10,
                outputTokens: 5,
                cacheReadTokens: 2,
                cacheCreationTokens: 1,
                totalCostUsd: "2.25",
                costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
                quota: null,
                quotaFetchState: null,
              },
            ],
            meteredProviders: [
              {
                provider: meteredWithSessions,
                eventCount: 2,
                inputTokens: 1,
                outputTokens: 2,
                cacheReadTokens: 1,
                cacheCreationTokens: 1,
                totalCostUsd: "1.25",
                costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
                quota: null,
                quotaFetchState: null,
              },
            ],
          },
        ],
      },
      isLoading: false,
    });

    render(<UsageDashboardPage />);
    const subscription = within(
      screen.getByTestId("subscription-provider-sub"),
    );
    expect(subscription.getByText("Proxy")).toBeInTheDocument();
    expect(subscription.getByText("18")).toBeInTheDocument();
    expect(subscription.getByText("4")).toBeInTheDocument();
    expect(subscription.getByText("2.25")).toBeInTheDocument();
    const metered = within(screen.getByTestId("metered-provider-api"));
    expect(metered.getByText("Session log")).toBeInTheDocument();
  });

  it("surfaces query, mutation, session warning and session error messages", async () => {
    mocks.dashboard.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("dashboard failed"),
    });
    mocks.providers.mockReturnValue({
      data: [subscriptionProvider, meteredProvider],
      isLoading: false,
      error: new Error("providers failed"),
    });
    mocks.bindings.mockReturnValue({
      data: [],
      error: new Error("bindings failed"),
    });
    mocks.events.mockReturnValue({
      data: undefined,
      error: new Error("events failed"),
    });
    const queries = render(<UsageDashboardPage />);
    for (const message of [
      "dashboard failed",
      "providers failed",
      "bindings failed",
    ]) {
      expect(screen.getByRole("alert", { name: message })).toBeInTheDocument();
    }
    queries.unmount();

    mocks.dashboard.mockReturnValue({
      data: {
        startAt: 10,
        endAt: 20,
        warnings: [],
        productGroups: [
          {
            productGroupId: "claude",
            inputTokens: 1,
            outputTokens: 1,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            totalCostUsd: null,
            costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
            tokenSources: ["proxy"],
            subscriptionProviders: [],
            meteredProviders: [
              {
                provider: meteredProvider,
                eventCount: 1,
                inputTokens: 1,
                outputTokens: 1,
                cacheReadTokens: 0,
                cacheCreationTokens: 0,
                totalCostUsd: null,
                costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
                quota: null,
                quotaFetchState: null,
              },
            ],
          },
        ],
      },
      isLoading: false,
    });
    const { unmount } = render(<UsageDashboardPage />);
    expect(
      screen.getByRole("alert", { name: "events failed" }),
    ).toBeInTheDocument();
    unmount();

    mocks.dashboard.mockReturnValue({
      data: {
        startAt: 10,
        endAt: 20,
        warnings: [],
        productGroups: [],
      },
      isLoading: false,
    });
    mocks.providers.mockReturnValue({
      data: [subscriptionProvider],
      isLoading: false,
    });
    mocks.bindings.mockReturnValue({ data: [], error: undefined });
    mocks.saveProvider.mockRejectedValueOnce(new Error("save failed"));
    const page = render(<UsageDashboardPage />);
    fireEvent.click(
      within(screen.getByTestId("provider-config-sub")).getByRole("button", {
        name: "Edit",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("save failed")).toHaveAttribute(
      "role",
      "alert",
    );
    page.unmount();

    mocks.setEnabled.mockRejectedValueOnce(new Error("enable failed"));
    mocks.startProxy.mockRejectedValueOnce(new Error("proxy failed"));
    const actions = render(<UsageDashboardPage />);
    fireEvent.click(screen.getByRole("button", { name: "Disable" }));
    fireEvent.click(screen.getByRole("button", { name: "Start proxy" }));
    expect(
      (await screen.findByText("enable failed")).closest("[role=alert]"),
    ).not.toBeNull();
    expect(
      (await screen.findByText("proxy failed")).closest("[role=alert]"),
    ).not.toBeNull();
    actions.unmount();

    mocks.syncSession.mockResolvedValueOnce({
      warnings: ["session warning"],
      errors: ["session error"],
    });
    mocks.dashboard.mockReturnValue({
      data: {
        startAt: 10,
        endAt: 20,
        warnings: [],
        productGroups: [
          {
            productGroupId: "claude",
            inputTokens: 0,
            outputTokens: 0,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            totalCostUsd: null,
            costSourceCounts: { upstream: 0, estimated: 0, unavailable: 0 },
            tokenSources: ["session_log"],
            subscriptionProviders: [
              {
                provider: subscriptionProvider,
                eventCount: 0,
                inputTokens: 0,
                outputTokens: 0,
                cacheReadTokens: 0,
                cacheCreationTokens: 0,
                totalCostUsd: null,
                costSourceCounts: { upstream: 0, estimated: 0, unavailable: 0 },
                quota: null,
                quotaFetchState: null,
              },
            ],
            meteredProviders: [],
          },
        ],
      },
      isLoading: false,
    });
    render(<UsageDashboardPage />);
    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));
    expect(await screen.findByText("session warning")).toBeInTheDocument();
    expect(
      (await screen.findByText("session error")).closest("[role=alert]"),
    ).not.toBeNull();
  });

  it("disables relevant actions while mutations are pending", () => {
    Object.keys(ui.pending).forEach((key) => {
      ui.pending[key as keyof typeof ui.pending] = true;
    });
    render(<UsageDashboardPage />);
    for (const label of [
      "Start proxy",
      "Save routes",
      "Disable",
      "Refresh quota",
      "Sync sessions",
    ]) {
      for (const button of screen.getAllByRole("button", { name: label })) {
        expect(button).toBeDisabled();
      }
    }
    fireEvent.click(
      within(screen.getByTestId("provider-config-sub")).getByRole("button", {
        name: "Edit",
      }),
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("surfaces proxy status errors and disables proxy control until status is known", () => {
    mocks.isRunning.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("proxy status failed"),
    });
    const failed = render(<UsageDashboardPage />);
    expect(
      screen.getByRole("alert", { name: "proxy status failed" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start proxy" })).toBeDisabled();
    failed.unmount();

    mocks.isRunning.mockReturnValue({
      data: undefined,
      isLoading: true,
      error: undefined,
    });
    render(<UsageDashboardPage />);
    expect(screen.getByRole("button", { name: "Start proxy" })).toBeDisabled();
  });

  it("localizes an unavailable recent-event cost source", () => {
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
    });
    render(<UsageDashboardPage />);
    expect(
      within(screen.getByTestId("metered-provider-api")).getByText(
        "费用不可用",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("unavailable")).not.toBeInTheDocument();
  });

  it("renders core dashboard labels in Chinese", () => {
    ui.language = "zh";
    mocks.events.mockReturnValue({
      data: {
        items: [
          {
            eventId: "one",
            model: "model",
            totalCostUsd: "1",
            costSource: "upstream",
          },
        ],
        total: 1,
        page: 1,
        pageSize: 5,
      },
    });
    render(<UsageDashboardPage />);
    for (const text of [
      "今天",
      "7 天",
      "30 天",
      "自定义范围",
      "会话日志",
      "本地代理",
      "令牌",
      "请求",
      "近期请求",
    ]) {
      expect(screen.getAllByText(text).length).toBeGreaterThan(0);
    }
  });
});
