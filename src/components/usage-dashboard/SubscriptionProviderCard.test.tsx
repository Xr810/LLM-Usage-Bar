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
      if (key === "manualResetCredits.available") {
        return `${options?.count} 次`;
      }
      if (key === "manualResetCredits.toggle") {
        return `展开或收起 ${options?.count} 次使用限额重置`;
      }
      if (key === "manualResetCredits.expiresAt") {
        return `将于 ${options?.value} 到期`;
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
      name: "ChatGPT",
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
      fiveHourPace: { status: "green" },
      sevenDayPace: { status: "unknown" },
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
    usage.provider.name = "Claude";
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

  it("keeps the window label readable when that window is unavailable", () => {
    // The unavailable copy is a sentence, not a figure. Putting it in the
    // right-aligned value slot starved the label down to "5 …" in the real app.
    // ChatGPT in the real app: no 5-hour window, a healthy weekly one.
    const usage = subscriptionUsage();
    usage.quota!.fiveHourUtilizationPercent = null;
    usage.quota!.fiveHourResetsAt = null;
    usage.quota!.sevenDayUtilizationPercent = "14";
    usage.quota!.sevenDayResetsAt = "2026-07-17T01:00:00.000Z";

    render(
      <SubscriptionProviderCard
        usage={usage}
        layout="compact"
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    // Label and explanation are separate elements, so neither can squeeze the other.
    const label = screen.getByText("5-hour window");
    const explanation = screen.getByText(
      "This subscription does not provide this quota window",
    );
    expect(label).toBeInTheDocument();
    expect(explanation).toBeInTheDocument();
    expect(label).not.toBe(explanation);
    expect(label.contains(explanation)).toBe(false);
    // The window with real data still shows its figure alongside its label.
    expect(screen.getByText("Weekly allowance")).toBeInTheDocument();
    // Only the window with data draws a bar; the unavailable one draws none.
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  });

  it("keeps Provider actions wired in the default layout", () => {
    const usage = subscriptionUsage();
    const onRefreshQuota = vi.fn().mockResolvedValue({});
    const onSyncSessions = vi.fn().mockResolvedValue(undefined);

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={onRefreshQuota}
        onSyncSessions={onSyncSessions}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Refresh quota" }));
    fireEvent.click(screen.getByRole("button", { name: "Sync sessions" }));

    expect(usage.provider.quotaSource).toBe("codex_oauth");
    expect(onRefreshQuota).toHaveBeenCalledWith("system-chatgpt-subscription");
    expect(onSyncSessions).toHaveBeenCalledWith("system-chatgpt-subscription");
  });

  it("separates Claude quota provenance from unverified Code token logs", () => {
    const usage = subscriptionUsage();
    usage.provider.id = "system-claude-subscription";
    usage.provider.name = "Claude";
    usage.provider.productGroupId = "claude-subscription";
    usage.provider.sessionSourceBindings = ["claude"];
    usage.provider.quotaSource = "claude_local";
    usage.quota!.sourceObservedAt = 1_234;
    usage.quota!.fetchedAt = 5_678;
    usage.quotaFetchState = {
      providerId: usage.provider.id,
      lastAttemptAt: 9_999,
      lastSuccessAt: 9_999,
      lastError: null,
      consecutiveFailures: 0,
      stale: false,
    };

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    // The line carries the freshness alone now; naming the sources and their
    // caveats on every render answered a question asked once.
    const provenance = screen.getByTestId("provider-provenance");
    expect(provenance).not.toHaveTextContent(/local sample|Claude Code log/);
    // It shows a relative age; the exact timestamp is the tooltip, and it must
    // come from the local sample (1_234) rather than the fetch attempt (9_999).
    expect(provenance).toHaveAttribute(
      "title",
      new Date(1_234 * 1_000).toLocaleString(),
    );
    expect(provenance).not.toHaveAttribute(
      "title",
      new Date(9_999 * 1_000).toLocaleString(),
    );
  });

  it("names the plan once the credential reports one, and keeps the renewal date on hover", () => {
    const usage = subscriptionUsage();
    usage.quota!.planType = "pro";
    // 2026-09-14T00:00:00Z
    usage.quota!.planRenewsAt = 1_789_344_000;

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    // The tier is part of what the account is called, not a separate fact
    // filed beside a name that looks unfinished.
    const heading = screen.getByRole("heading", { name: "ChatGPT Pro" });
    expect(heading).toHaveAttribute("title", expect.stringMatching(/2026/));
    // The badge marks billing kind, which is what separates this card from a
    // metered one — it is not where the tier goes.
    expect(screen.getByText("Subscription")).toBeInTheDocument();
  });

  it("leaves the name alone for a credential that reports no plan", () => {
    render(
      <SubscriptionProviderCard
        usage={subscriptionUsage()}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "ChatGPT" }),
    ).toBeInTheDocument();
  });

  it.each([
    { status: "green" as const, toneClass: "bg-success" },
    { status: "yellow" as const, toneClass: "bg-warning" },
    { status: "red" as const, toneClass: "bg-danger" },
  ])(
    "colours the quota bar from the backend $status verdict",
    ({ status, toneClass }) => {
      const usage = subscriptionUsage();
      usage.quota!.fiveHourUtilizationPercent = "50";
      usage.quota!.fiveHourPace.status = status;

      render(
        <SubscriptionProviderCard
          usage={usage}
          onRefreshQuota={vi.fn()}
          onSyncSessions={vi.fn()}
        />,
      );

      const meter = screen.getByRole("progressbar", { name: "5-hour window" });
      // The fill still tracks remaining quota; only the tone follows the verdict.
      expect(meter).toHaveAttribute("aria-valuenow", "50");
      expect(meter.firstElementChild).toHaveStyle({ width: "50%" });
      expect(meter.firstElementChild).toHaveClass(toneClass);
    },
  );

  it("keeps a nearly-spent window green when the backend says the reset is imminent", () => {
    const usage = subscriptionUsage();
    usage.quota!.fiveHourUtilizationPercent = "81";
    usage.quota!.fiveHourPace.status = "green";

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    const meter = screen.getByRole("progressbar", { name: "5-hour window" });
    expect(meter).toHaveAttribute("aria-valuenow", "19");
    // 19% left would be danger under the static bands; the pace verdict wins.
    expect(meter.firstElementChild).toHaveClass("bg-success");
  });

  it("falls back to the remaining-percent thresholds when no verdict came through", () => {
    const usage = subscriptionUsage();
    usage.quota!.fiveHourUtilizationPercent = "45";
    usage.quota!.fiveHourPace.status = "unknown";

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

  it("labels the secondary window as weekly allowance and expands manual reset credits", () => {
    const usage = subscriptionUsage();
    usage.quota!.sevenDayUtilizationPercent = "4";
    usage.quota!.sevenDayResetsAt = "2026-07-20T08:00:00.000Z";
    usage.quota!.manualResetsRemaining = 3;
    usage.quota!.manualResetCredits = [
      {
        id: "reset-1",
        title: "Full reset",
        expiresAt: "2026-07-27T00:00:00.000Z",
      },
      {
        id: "reset-2",
        title: "Full reset",
        expiresAt: "2026-08-01T00:00:00.000Z",
      },
      {
        id: "reset-3",
        title: "Full reset",
        expiresAt: "2026-08-13T00:00:00.000Z",
      },
    ];

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(screen.getByText("Weekly allowance")).toBeInTheDocument();
    expect(screen.queryByText("7-day window")).toBeNull();
    expect(screen.getByText("3 次")).toBeInTheDocument();
    expect(screen.queryByText("Full reset")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", {
        name: "展开或收起 3 次使用限额重置",
      }),
    );

    expect(screen.getAllByText("Full reset")).toHaveLength(3);
    expect(screen.getByText(/7月27日/)).toBeInTheDocument();
    expect(screen.getByText(/8月1日/)).toBeInTheDocument();
    expect(screen.getByText(/8月13日/)).toBeInTheDocument();
  });

  it("shows a count without offering a fake toggle when expiry details are unavailable", () => {
    const usage = subscriptionUsage();
    usage.quota!.manualResetsRemaining = 3;

    render(
      <SubscriptionProviderCard
        usage={usage}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(screen.getByText("3 次")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: "展开或收起 3 次使用限额重置",
      }),
    ).toBeNull();
  });

  it("keeps quota details but hides Provider actions in the compact layout", () => {
    render(
      <SubscriptionProviderCard
        usage={subscriptionUsage()}
        layout="compact"
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(
      screen.getByTestId("subscription-provider-system-chatgpt-subscription"),
    ).toHaveAttribute("data-layout", "compact");
    expect(screen.getByText("5-hour window")).toBeInTheDocument();
    expect(screen.getByText("Weekly allowance")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Refresh quota" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Sync sessions" })).toBeNull();
  });
});
