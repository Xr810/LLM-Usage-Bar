import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { server } from "../../../tests/msw/server";
import { commandCalls, emitTauriEvent } from "../../../tests/msw/tauriMocks";
import { TrayUsagePopover } from "./TrayUsagePopover";
import { TrayUsagePopoverView } from "./TrayUsagePopoverView";

const NOW = new Date("2026-07-16T12:00:00Z");
const snapshot: TrayUsageSnapshot = {
  buildType: "community",
  sources: [],
  status: "yellow",
  generatedAt: 1_784_199_600,
  lastSuccessAt: 1_784_203_140,
  stale: false,
  refreshError: null,
  refreshInProgress: false,
  apiBudget: {
    mode: "per_provider",
    providerCount: 1,
    todayCostUsd: "8",
    dailyBudgetUsd: null,
    budgetConsumedPercent: null,
    costQuality: "complete",
    status: "red",
    warningReason: null,
  },
  agents: [
    {
      agentModuleId: "providers",
      name: "Providers",
      sortOrder: 0,
      status: "yellow",
      providers: [
        {
          providerId: "system-chatgpt-subscription",
          providerName: "ChatGPT",
          systemPresetKey: "chatgpt-subscription",
          billingKind: "subscription",
          status: "yellow",
          warningReason: null,
          recentUsage: {
            startAt: 1_781_611_200,
            endAt: 1_784_203_200,
            todayTokens: 4_200_000,
            totalTokens: 87_000_000,
            todayCostUsd: "1.25",
            totalCostUsd: "48.5",
            costQuality: "estimated",
            mostUsedModel: "gpt-5.6-sol",
            trendBuckets: [
              {
                startAt: 1_784_116_800,
                endAt: 1_784_203_200,
                eventCount: 8,
                inputTokens: 1_000_000,
                outputTokens: 200_000,
                cacheReadTokens: 3_000_000,
                cacheCreationTokens: 0,
                totalTokens: 4_200_000,
                totalCostUsd: "1.25",
                costSourceCounts: {
                  upstream: 0,
                  estimated: 8,
                  unavailable: 0,
                },
              },
            ],
          },
          subscription: {
            planLabel: "Plus",
            windows: [
              {
                kind: "five_hour",
                usedPercent: "60",
                remainingPercent: "40",
                resetsAt: "2026-07-16T14:00:00Z",
                status: "yellow",
                unavailableReason: null,
              },
            ],
          },
          metered: null,
        },
        {
          providerId: "system-openai-api",
          providerName: "OpenAI API",
          systemPresetKey: "openai-api",
          billingKind: "metered",
          status: "green",
          warningReason: null,
          recentUsage: {
            startAt: 1_781_611_200,
            endAt: 1_784_203_200,
            todayTokens: 0,
            totalTokens: 2_300_000,
            todayCostUsd: "8",
            totalCostUsd: "20",
            costQuality: "complete",
            mostUsedModel: "gpt-4.1",
            trendBuckets: [
              {
                startAt: 1_784_116_800,
                endAt: 1_784_203_200,
                eventCount: 3,
                inputTokens: 500_000,
                outputTokens: 50_000,
                cacheReadTokens: 200_000,
                cacheCreationTokens: 0,
                totalTokens: 750_000,
                totalCostUsd: "8",
                costSourceCounts: {
                  upstream: 3,
                  estimated: 0,
                  unavailable: 0,
                },
              },
            ],
          },
          subscription: null,
          metered: {
            todayCostUsd: "8",
            rolling30DayCostUsd: "20",
            dailyBudgetUsd: "10",
            budgetConsumedPercent: "80",
            totalTokens: 2_300_000,
            costQuality: "complete",
          },
        },
      ],
    },
  ],
};

function props(
  overrides: Partial<React.ComponentProps<typeof TrayUsagePopoverView>> = {},
) {
  return {
    snapshot,
    loading: false,
    refreshing: false,
    onRefresh: vi.fn(),
    onOpenDetails: vi.fn(),
    onOpenSettings: vi.fn(),
    onQuit: vi.fn(),
    now: NOW,
    ...overrides,
  };
}

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("TrayUsagePopover Provider-only UI", () => {
  beforeAll(async () => {
    i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
    i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
    await i18n.changeLanguage("en");
  });

  it("renders Provider accounts directly without Agent tabs or labels", () => {
    render(<TrayUsagePopoverView {...props()} />);

    expect(screen.getByText(/Today.s API spend/)).toBeInTheDocument();
    expect(screen.getByText("ChatGPT Plus")).toBeInTheDocument();
    expect(screen.getByText("OpenAI API")).toBeInTheDocument();
    expect(screen.queryByText("$1.25")).toBeNull();
    expect(screen.queryByText("Most used model: gpt-5.6-sol")).toBeNull();
    expect(
      screen.queryAllByRole("img", {
        name: "Token usage for the last 30 days",
      }),
    ).toHaveLength(0);
    expect(screen.getByText("60%")).toBeInTheDocument();
    expect(screen.getByText("$8.00")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByText("Providers")).toBeNull();
    expect(screen.queryByText("Cap-enforced")).toBeNull();
    expect(screen.queryByText("Monitor only")).toBeNull();
    expect(document.querySelectorAll("[data-lub-marker]")).toHaveLength(0);
  });

  it("does not expand a per-account chart in the v5 tray", () => {
    render(<TrayUsagePopoverView {...props()} />);
    expect(screen.queryByRole("button", { name: "Usage details" })).toBeNull();
    expect(screen.queryByText("4.2M")).toBeNull();
  });

  it("keeps weekly windows and manual resets off the tray row", () => {
    const withExtraResets = structuredClone(snapshot);
    const subscription = withExtraResets.agents[0].providers[0].subscription!;
    subscription.windows.push({
      kind: "seven_day",
      usedPercent: "4",
      remainingPercent: "96",
      resetsAt: "2026-07-20T08:00:00Z",
      status: "green",
      unavailableReason: null,
    });
    subscription.manualResetsRemaining = 3;
    subscription.manualResetCredits = [
      {
        id: "reset-1",
        title: "Full reset",
        expiresAt: "2026-07-27T00:00:00Z",
      },
    ];

    render(<TrayUsagePopoverView {...props({ snapshot: withExtraResets })} />);

    expect(screen.queryByText("Weekly allowance")).toBeNull();
    expect(screen.queryByText("Full reset")).toBeNull();
    expect(screen.getByText("60%")).toBeInTheDocument();
  });

  it("shows subscription progress as used percent", () => {
    const mostlyAvailable = structuredClone(snapshot);
    const window =
      mostlyAvailable.agents[0].providers[0].subscription!.windows[0];
    window.usedPercent = "5";
    window.remainingPercent = "95";
    window.status = "green";

    render(<TrayUsagePopoverView {...props({ snapshot: mostlyAvailable })} />);

    expect(screen.getByText("5%")).toBeInTheDocument();
    expect(
      screen.getByRole("meter", { name: /ChatGPT Plus, monitor only/ }),
    ).toHaveAttribute("aria-valuenow", "5");
  });

  it("shows one combined API limit while keeping Provider cost breakdowns", () => {
    const shared = structuredClone(snapshot);
    shared.apiBudget = {
      mode: "shared",
      providerCount: 1,
      todayCostUsd: "8",
      dailyBudgetUsd: "20",
      budgetConsumedPercent: "40",
      costQuality: "complete",
      status: "green",
      warningReason: null,
    };
    shared.agents[0].providers[1].status = "unknown";
    shared.agents[0].providers[1].metered!.dailyBudgetUsd = null;
    shared.agents[0].providers[1].metered!.budgetConsumedPercent = null;

    render(<TrayUsagePopoverView {...props({ snapshot: shared })} />);

    expect(screen.getByText("/ $20")).toBeInTheDocument();
    expect(screen.getByText("OpenAI API")).toBeInTheDocument();
    expect(document.querySelectorAll("[data-lub-marker='day']")).toHaveLength(
      1,
    );
    expect(
      screen.queryByRole("progressbar", {
        name: "Daily budget for OpenAI API",
      }),
    ).toBeNull();
  });

  it("routes a missing combined limit to the shared budget settings", async () => {
    const user = userEvent.setup();
    const onOpenSettings = vi.fn();
    const shared = structuredClone(snapshot);
    shared.apiBudget = {
      mode: "shared",
      providerCount: 1,
      todayCostUsd: "8",
      dailyBudgetUsd: null,
      budgetConsumedPercent: null,
      costQuality: "complete",
      status: "unknown",
      warningReason: "daily_budget_missing",
    };

    render(
      <TrayUsagePopoverView {...props({ snapshot: shared, onOpenSettings })} />,
    );
    await user.click(
      screen.getByRole("button", { name: "Set combined API daily limit" }),
    );
    expect(onOpenSettings).toHaveBeenCalledWith(null);
  });

  it("routes Provider row and footer actions without Agent identity", async () => {
    const user = userEvent.setup();
    const onOpenDetails = vi.fn();
    const onOpenSettings = vi.fn();
    const withoutBudget = structuredClone(snapshot);
    withoutBudget.agents[0].providers[1].metered!.dailyBudgetUsd = null;
    withoutBudget.agents[0].providers[1].metered!.budgetConsumedPercent = null;
    render(
      <TrayUsagePopoverView
        {...props({ snapshot: withoutBudget, onOpenDetails, onOpenSettings })}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "Set daily budget for OpenAI API" }),
    );
    expect(onOpenSettings).toHaveBeenCalledWith("system-openai-api");

    await user.click(screen.getByRole("button", { name: /Trends/ }));
    expect(onOpenDetails).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenLastCalledWith(null);
  });

  it("renders loading and empty Provider states", () => {
    const { rerender } = render(
      <TrayUsagePopoverView {...props({ snapshot: null, loading: true })} />,
    );
    expect(screen.getByText("Loading usage…")).toBeInTheDocument();

    rerender(
      <TrayUsagePopoverView
        {...props({ snapshot: { ...snapshot, agents: [] } })}
      />,
    );
    expect(
      screen.getByText("No visible Provider accounts"),
    ).toBeInTheDocument();
  });

  it("opens the Provider dashboard from the live tray popover", async () => {
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    expect(await screen.findByText("ChatGPT Plus")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Trends/ }));
    await waitFor(() =>
      expect(commandCalls("open_main_from_tray").length).toBe(1),
    );
    expect(commandCalls("open_main_from_tray")[0]?.[1]).toEqual({
      destination: { kind: "usage", agentModuleId: null },
    });
  });

  it("does not draw 0% when a subscription has no quota window", () => {
    const empty = structuredClone(snapshot);
    empty.agents[0].providers[0].subscription!.windows = [];
    render(<TrayUsagePopoverView {...props({ snapshot: empty })} />);
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.getByText("Stale data")).toBeInTheDocument();
    expect(screen.queryByRole("meter", { name: /ChatGPT/ })).toBeNull();
    expect(
      screen.getByRole("group", {
        name: "ChatGPT Plus: no usage percentage, Stale data",
      }),
    ).toBeInTheDocument();
  });

  it("says the read failed when the refresh failed and the window is empty", () => {
    const failed = structuredClone(snapshot);
    failed.refreshError = "timeout";
    failed.agents[0].providers[0].subscription!.windows = [];
    render(<TrayUsagePopoverView {...props({ snapshot: failed })} />);
    expect(screen.getByText("Couldn't read usage")).toBeInTheDocument();
    expect(screen.queryByText("0%")).toBeNull();
    expect(
      screen.getByRole("group", {
        name: /ChatGPT Plus: no usage percentage, Couldn't read usage/,
      }),
    ).toBeInTheDocument();
  });

  it("treats a window unavailable reason as a failed read, not 0%", () => {
    const failed = structuredClone(snapshot);
    const window = failed.agents[0].providers[0].subscription!.windows[0];
    window.usedPercent = null;
    window.unavailableReason = "read_failed";
    render(<TrayUsagePopoverView {...props({ snapshot: failed })} />);
    expect(screen.getByText("Couldn't read usage")).toBeInTheDocument();
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.queryByRole("meter", { name: /ChatGPT/ })).toBeNull();
  });

  it("still shows a real zero percent window", () => {
    const zero = structuredClone(snapshot);
    zero.agents[0].providers[0].subscription!.windows[0].usedPercent = "0";
    render(<TrayUsagePopoverView {...props({ snapshot: zero })} />);
    expect(screen.getByText("0%")).toBeInTheDocument();
    expect(screen.queryByText("Stale data")).toBeNull();
  });

  it("ignores pace status and the deprecated warn level", () => {
    const quiet = structuredClone(snapshot);
    quiet.apiBudget.level = "ok";
    quiet.apiBudget.status = "red";
    quiet.apiBudget.budgetConsumedPercent = "100";
    const { rerender } = render(
      <TrayUsagePopoverView {...props({ snapshot: quiet })} />,
    );
    expect(screen.getByTestId("lub-tray-panel")).toHaveAttribute(
      "data-state",
      "ok",
    );

    const deprecated = structuredClone(snapshot);
    deprecated.apiBudget.level = "warn";
    deprecated.apiBudget.status = "green";
    deprecated.apiBudget.budgetConsumedPercent = "10";
    rerender(<TrayUsagePopoverView {...props({ snapshot: deprecated })} />);
    expect(screen.getByTestId("lub-tray-panel")).toHaveAttribute(
      "data-state",
      "ok",
    );
    expect(screen.queryByText(/Over today/)).toBeNull();
  });

  it("alerts when level is reached even if spend is under the cap", () => {
    const over = structuredClone(snapshot);
    over.apiBudget.level = "reached";
    over.apiBudget.mode = "shared";
    over.apiBudget.todayCostUsd = "3";
    over.apiBudget.dailyBudgetUsd = "10";
    over.apiBudget.budgetConsumedPercent = "30";
    over.apiBudget.status = "green";
    const { rerender } = render(
      <TrayUsagePopoverView {...props({ snapshot: over })} />,
    );
    expect(screen.getByTestId("lub-tray-panel")).toHaveAttribute(
      "data-state",
      "over",
    );
    expect(screen.getByText(/Over today.s \$10 cap/)).toBeInTheDocument();
    expect(
      screen.getByText(
        "Alert only. Nothing is paused; API requests keep going.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Over cap")).toBeInTheDocument();
    expect(screen.queryByText("Compare Free vs Pro")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Stop all API spend" }),
    ).toBeNull();
    const allow = screen.getAllByRole("button", { name: /Allow/ });
    expect(allow).toHaveLength(2);
    for (const button of allow) {
      expect(button).toHaveAttribute("aria-disabled", "true");
    }
    expect(screen.queryByText("Paused")).toBeNull();
    const slot = document.querySelector("[data-slot='community-allow']");
    expect(slot).not.toBeNull();
    expect(slot?.querySelectorAll("button")).toHaveLength(2);
    expect(slot?.querySelector("a, [href]")).toBeNull();
    expect(document.querySelector(".lub-meter-overflow")).toBeNull();

    const official = structuredClone(over);
    official.buildType = "official";
    rerender(<TrayUsagePopoverView {...props({ snapshot: official })} />);
    expect(
      document.querySelector("[data-slot='community-allow']"),
    ).not.toBeNull();

    const underTheLevel = structuredClone(over);
    underTheLevel.apiBudget.level = "ok";
    underTheLevel.apiBudget.todayCostUsd = "12";
    underTheLevel.apiBudget.budgetConsumedPercent = "120";
    underTheLevel.apiBudget.status = "red";
    rerender(<TrayUsagePopoverView {...props({ snapshot: underTheLevel })} />);
    expect(screen.getByTestId("lub-tray-panel")).toHaveAttribute(
      "data-state",
      "ok",
    );
    expect(screen.queryByText(/Over today/)).toBeNull();
    expect(screen.queryByText("Over cap")).toBeNull();
  });

  it("shows the Free vs Pro line only when upsellEntry is on", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json({
          tier: "free",
          state: "free",
          build: "official",
          features: [],
          uiFlags: { upsellEntry: false },
        }),
      ),
    );
    const over = structuredClone(snapshot);
    over.apiBudget.level = "reached";
    over.apiBudget.mode = "shared";
    over.apiBudget.todayCostUsd = "12";
    over.apiBudget.dailyBudgetUsd = "10";
    over.buildType = "official";
    const { rerender } = render(
      <TrayUsagePopoverView
        {...props({ snapshot: over, upsellEntry: false })}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector("[data-slot='community-allow']"),
      ).toBeNull();
    });
    expect(
      screen.queryByRole("button", { name: "Compare Free vs Pro" }),
    ).toBeNull();
    rerender(
      <TrayUsagePopoverView
        {...props({ snapshot: over, upsellEntry: true })}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Compare Free vs Pro" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Want spending to stop at the cap?"),
    ).toBeInTheDocument();
  });

  it("reads upsellEntry from get_license_status and keeps the community default off", async () => {
    const over = structuredClone(snapshot);
    over.apiBudget.level = "reached";
    over.apiBudget.mode = "shared";
    over.apiBudget.todayCostUsd = "12";
    over.apiBudget.dailyBudgetUsd = "10";
    over.buildType = "community";
    server.use(
      http.post("http://tauri.local/get_tray_usage_snapshot", () =>
        HttpResponse.json(over),
      ),
      http.post("http://tauri.local/refresh_tray_usage", () =>
        HttpResponse.json(over),
      ),
    );
    const { unmount } = render(<TrayUsagePopover />, { wrapper: wrapper() });
    expect(await screen.findByText(/Over today/)).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("get_license_status").length).toBeGreaterThan(0),
    );
    expect(
      screen.queryByRole("button", { name: "Compare Free vs Pro" }),
    ).toBeNull();
    const slot = document.querySelector("[data-slot='community-allow']");
    expect(slot).not.toBeNull();
    expect(slot?.querySelectorAll("button")).toHaveLength(2);
    expect(slot?.querySelector("a, [href]")).toBeNull();
    unmount();

    server.use(
      http.post("http://tauri.local/get_tray_usage_snapshot", () =>
        HttpResponse.json({ ...over, buildType: "official" }),
      ),
      http.post("http://tauri.local/refresh_tray_usage", () =>
        HttpResponse.json({ ...over, buildType: "official" }),
      ),
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json({
          tier: "pro",
          build: "official",
          features: [],
          uiFlags: { upsellEntry: true },
          expiresAt: "2026-12-01T00:00:00Z",
        }),
      ),
    );
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    expect(
      await screen.findByRole("button", { name: "Compare Free vs Pro" }),
    ).toBeInTheDocument();
    expect(document.querySelector("[data-slot='community-allow']")).toBeNull();
  });

  it("does not refresh when quota or guard events arrive without a snapshot change", async () => {
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    await screen.findByText("ChatGPT Plus");
    const before = commandCalls("refresh_tray_usage").length;
    act(() => {
      emitTauriEvent("quota-updated");
      emitTauriEvent("guard-state-changed");
    });
    expect(commandCalls("refresh_tray_usage").length).toBe(before);
  });

  it("refreshes on show and hides on Escape", async () => {
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    await screen.findByText("ChatGPT Plus");

    act(() => emitTauriEvent("tray-popover-shown"));
    await waitFor(() =>
      expect(commandCalls("refresh_tray_usage").length).toBeGreaterThan(0),
    );
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() =>
      expect(commandCalls("hide_tray_popover").length).toBe(1),
    );
  });
});
