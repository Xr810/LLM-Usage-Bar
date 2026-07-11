import {
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

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
    i18n: { resolvedLanguage: "en", language: "en" },
  }),
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageDashboard: (...args: unknown[]) => mocks.dashboard(...args),
  useUsageEvents: (...args: unknown[]) => mocks.events(...args),
  useUsageProviders: () => mocks.providers(),
  useRouteBindings: () => mocks.bindings(),
  useSaveUsageProvider: () => ({
    mutateAsync: mocks.saveProvider,
    isPending: false,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: mocks.setEnabled,
    isPending: false,
  }),
  useSetRouteBinding: () => ({
    mutateAsync: mocks.setBinding,
    isPending: false,
  }),
  useRefreshProviderQuota: () => ({
    mutateAsync: mocks.refreshQuota,
    isPending: false,
  }),
  useSyncProviderSessionUsage: () => ({
    mutateAsync: mocks.syncSession,
    isPending: false,
  }),
}));

vi.mock("@/lib/query/proxy", () => ({
  useIsProxyRunning: () => mocks.isRunning(),
  useStartProxyServer: () => ({
    mutateAsync: mocks.startProxy,
    isPending: false,
  }),
  useStopProxyServer: () => ({
    mutateAsync: mocks.stopProxy,
    isPending: false,
  }),
}));

vi.mock("@/components/usage/UsageDateRangePicker", () => ({
  UsageDateRangePicker: ({
    onApply,
  }: {
    onApply: (value: unknown) => void;
  }) => (
    <button
      type="button"
      onClick={() =>
        onApply({ preset: "custom", customStartDate: 100, customEndDate: 200 })
      }
    >
      Custom range
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

  it("supports today, 7d, 30d and custom ranges", () => {
    render(<UsageDashboardPage />);
    for (const label of ["Today", "7 days", "30 days", "Custom range"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "7 days" }));
    fireEvent.click(screen.getByRole("button", { name: "30 days" }));
    fireEvent.click(screen.getByRole("button", { name: "Custom range" }));
    expect(mocks.dashboard).toHaveBeenCalled();
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
});
