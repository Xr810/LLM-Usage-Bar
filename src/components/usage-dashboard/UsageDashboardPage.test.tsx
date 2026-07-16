import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentModuleView,
  UsageDashboardView,
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
  onUsageRecorded: undefined as undefined | (() => void),
  refreshPending: false,
  syncPending: false,
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

function agent(id: string, name: string): AgentModuleView {
  return {
    id,
    name,
    sortOrder: 1,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 2,
  };
}

const codex = agent("codex", "Codex");
const claude = agent("claude-code", "Claude Code");

function provider(
  id: string,
  billingKind: "subscription" | "metered",
): UsageProviderView {
  return {
    id,
    name: id === "sub" ? "Official Subscription" : "Metered API",
    billingKind,
    productGroupId: "codex",
    tokenSources: billingKind === "subscription" ? ["session_log"] : ["proxy"],
    sessionSourceBindings: billingKind === "subscription" ? ["codex"] : [],
    bindings: [],
    quotaSource: billingKind === "subscription" ? "codex_oauth" : null,
    quotaIntervalSeconds: billingKind === "subscription" ? 300 : null,
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
  };
}

function dashboardData(agentModuleId = codex.id): UsageDashboardView {
  return {
    agentModuleId,
    startAt: 10,
    endAt: 20,
    warnings: [],
    productGroups: [
      {
        productGroupId: "codex",
        inputTokens: 100,
        outputTokens: 20,
        cacheReadTokens: 5,
        cacheCreationTokens: 3,
        totalCostUsd: "1.25",
        costSourceCounts: { upstream: 1, estimated: 0, unavailable: 1 },
        tokenSources: ["proxy", "session_log"],
        subscriptionProviders: [
          {
            provider: provider("sub", "subscription"),
            sharedAccount: true,
            eventCount: 1,
            inputTokens: 60,
            outputTokens: 10,
            cacheReadTokens: 5,
            cacheCreationTokens: 0,
            totalCostUsd: null,
            costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
            quota: null,
            quotaFetchState: null,
          },
        ],
        meteredProviders: [
          {
            provider: provider("metered", "metered"),
            sharedAccount: false,
            eventCount: 2,
            inputTokens: 40,
            outputTokens: 10,
            cacheReadTokens: 0,
            cacheCreationTokens: 3,
            totalCostUsd: "1.25",
            costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
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

  it("renders one Agent's subscription and metered sections together", () => {
    render(<UsageDashboardPage selectedAgent={codex} />);

    expect(mocks.dashboard).toHaveBeenCalledWith(
      "codex",
      expect.any(Number),
      expect.any(Number),
    );
    expect(screen.getByText("Official Subscription")).toBeInTheDocument();
    expect(screen.getByText("Metered API")).toBeInTheDocument();
    expect(screen.getByText("Subscription accounts")).toBeInTheDocument();
    expect(screen.getByText("Metered usage")).toBeInTheDocument();
  });

  it("never renders prior-Agent data after a selection switch", () => {
    const { rerender } = render(<UsageDashboardPage selectedAgent={codex} />);
    expect(screen.getByText("Official Subscription")).toBeInTheDocument();

    rerender(<UsageDashboardPage selectedAgent={claude} />);

    expect(mocks.dashboard).toHaveBeenLastCalledWith(
      "claude-code",
      expect.any(Number),
      expect.any(Number),
    );
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.queryByText("Metered API")).toBeNull();
  });

  it("clears prior-Agent quota and session results after a selection switch", async () => {
    mocks.refreshQuota.mockRejectedValueOnce(new Error("codex refresh failed"));
    mocks.syncSession.mockResolvedValueOnce({
      warnings: ["codex session warning"],
      errors: ["codex session error"],
    });
    const { rerender } = render(<UsageDashboardPage selectedAgent={codex} />);

    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));
    expect(
      await screen.findByText("codex session warning"),
    ).toBeInTheDocument();
    expect(await screen.findByText("codex session error")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
    expect(await screen.findByText("codex refresh failed")).toBeInTheDocument();

    rerender(<UsageDashboardPage selectedAgent={claude} />);

    expect(screen.queryByText("codex refresh failed")).toBeNull();
    expect(screen.queryByText("codex session warning")).toBeNull();
    expect(screen.queryByText("codex session error")).toBeNull();
  });

  it("supports exact today, 7d, 30d and custom Agent-scoped ranges", () => {
    vi.useFakeTimers();
    const nowMs = new Date("2026-07-12T12:34:56.789Z").getTime();
    vi.setSystemTime(nowMs);
    try {
      render(<UsageDashboardPage selectedAgent={codex} />);
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
        "codex",
        todayStart,
        endDate,
      );

      fireEvent.click(screen.getByRole("button", { name: "7 days" }));
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        "codex",
        expect.any(Number),
        endDate,
      );
      fireEvent.click(screen.getByRole("button", { name: "30 days" }));
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        "codex",
        expect.any(Number),
        endDate,
      );
      fireEvent.click(screen.getByRole("button", { name: "Custom range" }));
      expect(mocks.dashboard).toHaveBeenLastCalledWith("codex", 100, 200);
    } finally {
      vi.useRealTimers();
    }
  });

  it("advances a live Agent range on invalidation events", () => {
    vi.useFakeTimers();
    const initialMs = new Date("2026-07-12T12:34:56.100Z").getTime();
    vi.setSystemTime(initialMs);
    try {
      render(<UsageDashboardPage selectedAgent={codex} />);
      vi.setSystemTime(initialMs + 2_000);
      act(() => ui.onUsageRecorded?.());
      expect(mocks.dashboard).toHaveBeenLastCalledWith(
        "codex",
        expect.any(Number),
        Math.floor((initialMs + 2_000) / 1000) + 1,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("surfaces dashboard, quota, and session errors without hiding the Agent", async () => {
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
    render(<UsageDashboardPage selectedAgent={codex} />);

    expect(screen.getByText("partial dashboard")).toBeInTheDocument();
    expect(
      screen.getByRole("alert", { name: "dashboard failed" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
    expect(await screen.findByText("refresh failed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));
    expect(await screen.findByText("session warning")).toBeInTheDocument();
    expect(await screen.findByText("session error")).toBeInTheDocument();
  });

  it("replaces repeated quota errors and clears them after a successful refresh", async () => {
    mocks.dashboard.mockReturnValue({
      data: dashboardData(),
      isLoading: false,
      error: new Error("dashboard failed"),
    });
    mocks.refreshQuota
      .mockRejectedValueOnce(new Error("refresh failed"))
      .mockRejectedValueOnce(new Error("refresh failed"))
      .mockResolvedValueOnce({});
    render(<UsageDashboardPage selectedAgent={codex} />);

    const refreshButton = screen.getByRole("button", {
      name: "Refresh quota",
    });

    await act(async () => {
      fireEvent.click(refreshButton);
      await Promise.resolve();
    });
    expect(
      screen.getAllByRole("alert", { name: "refresh failed" }),
    ).toHaveLength(1);

    await act(async () => {
      fireEvent.click(refreshButton);
      await Promise.resolve();
    });
    expect(
      screen.getAllByRole("alert", { name: "refresh failed" }),
    ).toHaveLength(1);

    await act(async () => {
      fireEvent.click(refreshButton);
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("alert", { name: "refresh failed" }),
      ).toBeNull(),
    );
    expect(
      screen.getByRole("alert", { name: "dashboard failed" }),
    ).toBeInTheDocument();
  });

  it("deduplicates matching query and action errors", async () => {
    mocks.dashboard.mockReturnValue({
      data: dashboardData(),
      isLoading: false,
      error: new Error("shared failure"),
    });
    mocks.refreshQuota.mockRejectedValueOnce(new Error("shared failure"));
    render(<UsageDashboardPage selectedAgent={codex} />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
      await Promise.resolve();
    });

    expect(
      screen.getAllByRole("alert", { name: "shared failure" }),
    ).toHaveLength(1);
  });
});
