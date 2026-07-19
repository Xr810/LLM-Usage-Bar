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
import { describe, expect, it, vi } from "vitest";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { commandCalls, emitTauriEvent } from "../../../tests/msw/tauriMocks";
import { TrayUsagePopover } from "./TrayUsagePopover";
import { TrayUsagePopoverView } from "./TrayUsagePopoverView";

const NOW = new Date("2026-07-16T12:00:00Z");
const snapshot: TrayUsageSnapshot = {
  status: "yellow",
  generatedAt: 1_784_199_600,
  lastSuccessAt: 1_784_203_140,
  stale: false,
  refreshError: null,
  refreshInProgress: false,
  agents: [
    {
      agentModuleId: "providers",
      name: "Providers",
      sortOrder: 0,
      status: "yellow",
      providers: [
        {
          providerId: "system-chatgpt-subscription",
          providerName: "ChatGPT Plus/Pro",
          systemPresetKey: "chatgpt-subscription",
          billingKind: "subscription",
          status: "yellow",
          warningReason: null,
          recentUsage: {
            startAt: 1_781_611_200,
            endAt: 1_784_203_200,
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
  it("renders Provider accounts directly without Agent tabs or labels", () => {
    render(<TrayUsagePopoverView {...props()} />);

    expect(
      screen.getByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(screen.getByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    expect(screen.getByText("OpenAI API")).toBeInTheDocument();
    expect(
      screen.getByText("Most used model: gpt-5.6-sol"),
    ).toBeInTheDocument();
    expect(
      screen.getAllByRole("img", {
        name: "Token usage for the last 30 days",
      }),
    ).toHaveLength(2);
    expect(
      screen.queryByText(/Estimated from this Provider/),
    ).not.toBeInTheDocument();
    expect(screen.getByText("$8.00 of $10.00")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByText("Providers")).toBeNull();
    expect(
      screen.getByRole("progressbar", {
        name: "Daily budget for OpenAI API",
      }),
    ).toHaveAttribute("aria-valuenow", "80");
  });

  it("uses the weekly label and expands GPT additional reset times", async () => {
    const user = userEvent.setup();
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
    subscription.additionalResetDetails = [
      {
        id: "0:604800",
        label: "Codex Spark",
        windowSeconds: 604_800,
        resetsAt: "2026-07-20T08:00:00Z",
      },
    ];

    render(<TrayUsagePopoverView {...props({ snapshot: withExtraResets })} />);

    expect(screen.getByText("Weekly allowance")).toBeInTheDocument();
    expect(screen.queryByText("7-day allowance")).toBeNull();
    expect(screen.queryByText(/Codex Spark/)).toBeNull();

    await user.click(
      screen.getByRole("button", {
        name: "Toggle 1 additional quota reset times",
      }),
    );

    expect(screen.getByText(/Codex Spark/)).toBeInTheDocument();
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

    await user.click(screen.getByRole("button", { name: "Open details" }));
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
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open details" }));
    await waitFor(() =>
      expect(commandCalls("open_main_from_tray").length).toBe(1),
    );
    expect(commandCalls("open_main_from_tray")[0]?.[1]).toEqual({
      destination: { kind: "usage", agentModuleId: null },
    });
  });

  it("refreshes on show and hides on Escape", async () => {
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    await screen.findByText("ChatGPT Plus/Pro");

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
