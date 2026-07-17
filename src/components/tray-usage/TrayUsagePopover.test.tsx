import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    expect(screen.getByText("$8.00 of $10.00")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByText("Providers")).toBeNull();
    expect(
      screen.getByRole("progressbar", {
        name: "Daily budget for OpenAI API",
      }),
    ).toHaveAttribute("aria-valuenow", "80");
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
    expect(screen.getByText("No visible Provider accounts")).toBeInTheDocument();
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
