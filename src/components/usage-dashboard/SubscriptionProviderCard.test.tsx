import { fireEvent, render, screen } from "@testing-library/react";
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
      id: "system-chatgpt-subscription",
      name: "ChatGPT Plus/Pro",
      billingKind: "subscription",
      productGroupId: "codex",
      tokenSources: ["session_log"],
      sessionSourceBindings: ["codex"],
      bindings: [],
      quotaSource: "codex_oauth",
      quotaIntervalSeconds: 300,
      dailyBudgetUsd: null,
      routeAppType: null,
      enabled: true,
      needsReview: false,
      createdAt: 1,
      updatedAt: 1,
      routeBaseUrl: null,
      hasRouteCredentials: false,
      systemPresetKey: null,
      systemAuthKind: null,
      canonicalEndpoint: null,
      compatibleAgentModuleIds: [],
      upstreamCredentialStatus: "not_required",
      upstreamCredentialVersion: 0,
      canClearUpstreamCredential: false,
      lastConnectionTestAt: null,
      lastConnectionTestStatus: null,
    },
    sharedAccount: false,
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

  it("ignores stale quota payloads when quota collection is unavailable", () => {
    const usage = subscriptionUsage();
    usage.provider.id = "system-claude-subscription";
    usage.provider.name = "Claude Pro/Max";
    usage.provider.productGroupId = "claude-subscription";
    usage.provider.sessionSourceBindings = ["claude"];
    usage.provider.quotaSource = null;
    usage.provider.quotaIntervalSeconds = null;
    usage.provider.systemPresetKey = "claude-subscription";
    usage.provider.systemAuthKind = "claude_cli";
    usage.quota = {
      ...usage.quota!,
      fiveHourUtilizationPercent: "25",
      sevenDayUtilizationPercent: "70",
    };
    usage.quotaFetchState = {
      providerId: usage.provider.id,
      lastAttemptAt: 10,
      lastSuccessAt: 9,
      lastError: "stale quota error",
      consecutiveFailures: 5,
      stale: true,
    };

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(
      screen.getAllByText(
        "This subscription does not provide this quota window",
      ),
    ).toHaveLength(2);
    expect(screen.queryByText("25% used")).toBeNull();
    expect(screen.queryByText("70% used")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Refresh quota" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Sync sessions" }),
    ).toBeInTheDocument();
  });

  it("refreshes ChatGPT quota through the managed Codex OAuth source", () => {
    const usage = subscriptionUsage();
    const onRefreshQuota = vi.fn().mockResolvedValue({});

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={onRefreshQuota}
        onSyncSessions={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));

    expect(usage.provider.quotaSource).toBe("codex_oauth");
    expect(onRefreshQuota).toHaveBeenCalledWith("system-chatgpt-subscription");
  });

  it.each([
    { used: "1", remaining: 99, toneClass: "bg-success" },
    { used: "50", remaining: 50, toneClass: "bg-warning" },
    { used: "81", remaining: 19, toneClass: "bg-danger" },
  ])(
    "fills the quota bar to $remaining% remaining with the matching status color",
    ({ used, remaining, toneClass }) => {
      const usage = subscriptionUsage();
      usage.quota!.fiveHourUtilizationPercent = used;

      render(
        <SubscriptionProviderCard
          usage={usage}
          onRefreshQuota={vi.fn()}
          onSyncSessions={vi.fn()}
        />,
      );

      const meter = screen.getByRole("progressbar", { name: "5-hour window" });
      expect(meter).toHaveAttribute("aria-valuenow", String(remaining));
      expect(meter.firstElementChild).toHaveStyle({ width: `${remaining}%` });
      expect(meter.firstElementChild).toHaveClass(toneClass);
    },
  );

  it("uses custom remaining thresholds for the quota color", () => {
    const usage = subscriptionUsage();
    usage.quota!.fiveHourUtilizationPercent = "45";

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
        remainingThresholds={{ warning: 60, critical: 30 }}
      />,
    );

    const meter = screen.getByRole("progressbar", { name: "5-hour window" });
    expect(meter).toHaveAttribute("aria-valuenow", "55");
    expect(meter.firstElementChild).toHaveClass("bg-warning");
  });
});
