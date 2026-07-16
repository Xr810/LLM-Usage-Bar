import { StrictMode, useState, type ReactNode } from "react";
import { readFile } from "node:fs/promises";
import path from "node:path";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import {
  afterAll,
  afterEach,
  beforeAll,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { TrayUsagePopover } from "./TrayUsagePopover";
import { TrayUsagePopoverView } from "./TrayUsagePopoverView";
import { TrayUsageProgress } from "./TrayUsageProgress";
import {
  emitTauriEvent,
  commandCalls,
  tauriInvoke,
} from "../../../tests/msw/tauriMocks";
import { server } from "../../../tests/msw/server";

const NOW = new Date("2026-07-16T12:00:00Z");
const styleElement = document.createElement("style");

beforeAll(async () => {
  const cssPath = path.resolve(process.cwd(), "src/index.css");
  const source = await readFile(cssPath, "utf8");
  const trayStylesStart = source.indexOf(
    'html[data-window-surface="tray-popover"]',
  );
  if (trayStylesStart < 0) {
    throw new Error("Tray popover production stylesheet was not found");
  }
  styleElement.textContent = source.slice(trayStylesStart);
  document.head.append(styleElement);
});

afterAll(() => {
  styleElement.remove();
});

afterEach(() => {
  document.documentElement.classList.remove("dark");
  document.documentElement.removeAttribute("data-window-surface");
});

const fullSnapshot: TrayUsageSnapshot = {
  status: "red",
  generatedAt: 1_784_199_600,
  lastSuccessAt: 1_784_203_140,
  stale: false,
  refreshError: null,
  refreshInProgress: false,
  agents: [
    {
      agentModuleId: "codex",
      name: "Codex",
      sortOrder: 1,
      status: "red",
      providers: [
        {
          providerId: "system-chatgpt-subscription",
          providerName: "ChatGPT Plus/Pro",
          systemPresetKey: "chatgpt-subscription",
          billingKind: "subscription",
          status: "yellow",
          warningReason: "backend_reason_must_not_render",
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
              {
                kind: "seven_day",
                usedPercent: "50",
                remainingPercent: "50",
                resetsAt: "2026-07-20T12:00:00Z",
                status: "yellow",
                unavailableReason: "raw_reason_must_not_render",
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
          status: "red",
          warningReason: null,
          subscription: null,
          metered: {
            todayCostUsd: "8",
            rolling30DayCostUsd: "2046.64",
            dailyBudgetUsd: "10",
            budgetConsumedPercent: "80",
            totalTokens: 2_300_000_000,
            costQuality: "partial",
          },
        },
      ],
    },
    {
      agentModuleId: "claude-code",
      name: "Claude Code with a deliberately long account name",
      sortOrder: 2,
      status: "unknown",
      providers: [
        {
          providerId: "custom-metered",
          providerName: "Custom Meter",
          systemPresetKey: null,
          billingKind: "metered",
          status: "unknown",
          warningReason: "daily_budget_missing",
          subscription: null,
          metered: {
            todayCostUsd: "1.25",
            rolling30DayCostUsd: "20",
            dailyBudgetUsd: "",
            budgetConsumedPercent: null,
            totalTokens: 25_000,
            costQuality: "estimated",
          },
        },
        {
          providerId: "budgeted-offline",
          providerName: "Budgeted Offline",
          systemPresetKey: "anthropic-api",
          billingKind: "metered",
          status: "unknown",
          warningReason: "cost_unavailable",
          subscription: null,
          metered: {
            todayCostUsd: "1",
            rolling30DayCostUsd: "2",
            dailyBudgetUsd: "12",
            budgetConsumedPercent: "8",
            totalTokens: 0,
            costQuality: "unavailable",
          },
        },
      ],
    },
  ],
};

function viewProps(
  overrides: Partial<React.ComponentProps<typeof TrayUsagePopoverView>> = {},
) {
  return {
    snapshot: fullSnapshot,
    loading: false,
    selectedAgentId: "overview" as const,
    onSelectAgent: vi.fn(),
    refreshing: false,
    onRefresh: vi.fn(),
    onOpenDetails: vi.fn(),
    onOpenSettings: vi.fn(),
    onQuit: vi.fn(),
    now: NOW,
    ...overrides,
  };
}

function createQueryWrapper() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

async function settleLastCommand(command: string) {
  await waitFor(() => expect(commandCalls(command).length).toBeGreaterThan(0));
  let index = -1;
  for (
    let current = tauriInvoke.mock.calls.length - 1;
    current >= 0;
    current -= 1
  ) {
    if (tauriInvoke.mock.calls[current]?.[0] === command) {
      index = current;
      break;
    }
  }
  if (index < 0) throw new Error(`No ${command} invocation was recorded`);
  await tauriInvoke.mock.results[index]?.value.catch(() => undefined);
}

function parseColor(value: string): [number, number, number] {
  const hex = value.trim().match(/^#([\da-f]{6})$/i);
  if (hex) {
    const numeric = Number.parseInt(hex[1], 16);
    return [(numeric >> 16) & 255, (numeric >> 8) & 255, numeric & 255];
  }

  const rgb = value.match(
    /^rgba?\(\s*([\d.]+)[, ]+\s*([\d.]+)[, ]+\s*([\d.]+)/i,
  );
  if (!rgb) throw new Error(`Unsupported computed color: ${value}`);
  return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
}

function relativeLuminance(value: string): number {
  const [red, green, blue] = parseColor(value).map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrastRatio(first: string, second: string): number {
  const lighter = Math.max(relativeLuminance(first), relativeLuminance(second));
  const darker = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (lighter + 0.05) / (darker + 0.05);
}

function minimumPadding(element: Element): number {
  const style = getComputedStyle(element);
  return Math.min(
    Number.parseFloat(style.paddingTop),
    Number.parseFloat(style.paddingRight),
    Number.parseFloat(style.paddingBottom),
    Number.parseFloat(style.paddingLeft),
  );
}

describe("TrayUsagePopoverView", () => {
  it("uses the complete exact 380 by 520 frame without clipped outer decoration", () => {
    document.documentElement.dataset.windowSurface = "tray-popover";
    render(<TrayUsagePopoverView {...viewProps()} />);

    const frame = screen.getByTestId("tray-usage-popover");
    const style = getComputedStyle(frame);
    expect(style.width).toBe("380px");
    expect(style.height).toBe("520px");
    expect(style.marginTop).toBe("0px");
    expect(style.marginRight).toBe("0px");
    expect(style.marginBottom).toBe("0px");
    expect(style.marginLeft).toBe("0px");
    expect(style.boxShadow).toBe("none");
    expect(style.borderWidth).toBe("1px");
  });

  it("renders the reference hierarchy and semantic metrics", async () => {
    render(<TrayUsagePopoverView {...viewProps()} />);

    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    expect(screen.getByText("50% remaining")).toBeInTheDocument();
    expect(screen.getByText("$8.00 of $10.00")).toBeInTheDocument();
    expect(screen.getAllByText("Critical").length).toBeGreaterThan(0);
    expect(screen.getByText("Partial data")).toBeInTheDocument();
    const sevenDayProgress = screen.getByRole("progressbar", {
      name: "7-day allowance for ChatGPT Plus/Pro · Codex",
    });
    expect(sevenDayProgress).toHaveAttribute("aria-valuenow", "50");
    expect(sevenDayProgress).toHaveClass("tray-usage-progress", "is-yellow");
    expect(screen.getByText("2.3B")).toBeInTheDocument();
    expect(screen.getByText("CM")).toBeInTheDocument();
    expect(
      screen.getByRole("tab", {
        name: "Claude Code with a deliberately long account name",
      }),
    ).toHaveAttribute(
      "title",
      "Claude Code with a deliberately long account name",
    );
    expect(
      screen.queryByRole("progressbar", {
        name: "Daily budget Budgeted Offline",
      }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("ChatGPT Plus/Pro · Codex"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("backend_reason_must_not_render"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("raw_reason_must_not_render"),
    ).not.toBeInTheDocument();
    expect(screen.getAllByRole("status")).toHaveLength(1);
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Overview" })).toHaveFocus(),
    );
  });

  it("filters the local snapshot by Agent without invoking the backend", async () => {
    const user = userEvent.setup();
    const onSelectAgent = vi.fn();
    const { rerender } = render(
      <TrayUsagePopoverView
        {...viewProps({ onSelectAgent })}
        selectedAgentId="overview"
      />,
    );

    await user.click(screen.getByRole("tab", { name: "Codex" }));
    expect(onSelectAgent).toHaveBeenCalledWith("codex");

    rerender(
      <TrayUsagePopoverView
        {...viewProps({ onSelectAgent })}
        selectedAgentId="codex"
      />,
    );
    expect(screen.getByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    expect(screen.queryByText("Custom Meter")).not.toBeInTheDocument();
    expect(
      screen.getByRole("progressbar", {
        name: "5-hour allowance for ChatGPT Plus/Pro",
      }),
    ).toBeInTheDocument();
    expect(commandCalls("get_tray_usage_snapshot")).toHaveLength(0);
  });

  it("routes row actions with exact identities and footer actions without guessing", async () => {
    const user = userEvent.setup();
    const onOpenDetails = vi.fn();
    const onOpenSettings = vi.fn();
    const onRefresh = vi.fn();
    const onQuit = vi.fn();
    render(
      <TrayUsagePopoverView
        {...viewProps({
          selectedAgentId: "claude-code",
          onOpenDetails,
          onOpenSettings,
          onRefresh,
          onQuit,
        })}
      />,
    );

    await user.click(
      screen.getByRole("button", {
        name: "Set daily budget for Custom Meter",
      }),
    );
    expect(onOpenSettings).toHaveBeenLastCalledWith("custom-metered");

    await user.click(
      screen.getByRole("button", {
        name: "Open Budgeted Offline details",
      }),
    );
    expect(onOpenDetails).toHaveBeenLastCalledWith("claude-code");

    await user.click(screen.getByRole("button", { name: "Open details" }));
    expect(onOpenDetails).toHaveBeenLastCalledWith("claude-code");
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenLastCalledWith(null);
    await user.click(screen.getByRole("button", { name: "Quit" }));
    expect(onQuit).toHaveBeenCalledTimes(1);
  });

  it("renders loading, empty, and unknown states without fake values", () => {
    const { rerender } = render(
      <TrayUsagePopoverView
        {...viewProps({ snapshot: null, loading: true })}
      />,
    );
    expect(screen.getByTestId("tray-usage-popover")).toHaveAttribute(
      "aria-busy",
      "true",
    );
    expect(screen.getByText("Loading usage…")).toBeInTheDocument();

    rerender(
      <TrayUsagePopoverView
        {...viewProps({
          snapshot: { ...fullSnapshot, status: "unknown", agents: [] },
          loading: false,
        })}
      />,
    );
    expect(screen.getByText("No visible usage sources")).toBeInTheDocument();
    expect(screen.getAllByText("Data unavailable").length).toBeGreaterThan(0);
  });

  it("shows stale generic copy and never renders raw refresh errors", () => {
    render(
      <TrayUsagePopoverView
        {...viewProps({
          snapshot: {
            ...fullSnapshot,
            stale: true,
            refreshError: "oauth_token=must-never-render",
          },
        })}
      />,
    );

    expect(screen.getByText("Stale")).toHaveClass("text-foreground");
    expect(
      screen.getByText("Refresh failed; showing last successful data."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/oauth_token/)).not.toBeInTheDocument();
    expect(screen.getByText(/Updated /)).toBeInTheDocument();
  });

  it("disables and visibly marks refresh while refreshing", () => {
    render(<TrayUsagePopoverView {...viewProps({ refreshing: true })} />);

    const refresh = screen.getByRole("button", { name: "Refreshing" });
    expect(refresh).toBeDisabled();
    expect(refresh.querySelector("svg")).toHaveClass("animate-spin");
  });

  it("traverses tabs, row actions, and footer actions with the keyboard without clipped focus space", async () => {
    const user = userEvent.setup();
    function StatefulPopover() {
      const [selectedAgentId, setSelectedAgentId] = useState<
        "overview" | string
      >("overview");
      return (
        <TrayUsagePopoverView
          {...viewProps({ selectedAgentId, onSelectAgent: setSelectedAgentId })}
        />
      );
    }

    render(<StatefulPopover />);
    const overview = screen.getByRole("tab", { name: "Overview" });
    await waitFor(() => expect(overview).toHaveFocus());
    expect(minimumPadding(screen.getByRole("tablist"))).toBeGreaterThanOrEqual(
      4,
    );

    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Codex" })).toHaveFocus();
    expect(screen.getByRole("heading", { name: "Codex" })).toBeInTheDocument();
    await user.keyboard("{ArrowRight}");
    expect(
      screen.getByRole("tab", {
        name: "Claude Code with a deliberately long account name",
      }),
    ).toHaveFocus();

    await user.tab();
    const setBudget = screen.getByRole("button", {
      name: "Set daily budget for Custom Meter",
    });
    expect(setBudget).toHaveFocus();
    expect(
      minimumPadding(setBudget.closest("article")!),
    ).toBeGreaterThanOrEqual(4);
    await user.tab();
    expect(
      screen.getByRole("button", { name: "Open Budgeted Offline details" }),
    ).toHaveFocus();

    const footer = screen.getByRole("contentinfo");
    expect(minimumPadding(footer)).toBeGreaterThanOrEqual(4);
    for (const name of ["Open details", "Refresh", "Settings", "Quit"]) {
      await user.tab();
      expect(screen.getByRole("button", { name })).toHaveFocus();
    }

    for (const tab of screen.getAllByRole("tab")) {
      const panelId = tab.getAttribute("aria-controls");
      expect(panelId).toBeTruthy();
      expect(document.getElementById(panelId!)).toHaveAttribute(
        "role",
        "tabpanel",
      );
    }
  });

  it.each(["light", "dark"] as const)(
    "computes accessible %s progress boundaries and selected-tab text",
    (theme) => {
      document.documentElement.classList.toggle("dark", theme === "dark");
      render(
        <>
          <TrayUsagePopoverView {...viewProps()} />
          <TrayUsageProgress label="Cyan test" value={40} status="green" />
          <TrayUsageProgress label="Yellow test" value={60} status="yellow" />
        </>,
      );

      const selectedTab = screen.getByRole("tab", { name: "Overview" });
      const selectedTabStyle = getComputedStyle(selectedTab);
      expect(
        contrastRatio(selectedTabStyle.color, selectedTabStyle.backgroundColor),
      ).toBeGreaterThanOrEqual(4.5);

      for (const label of ["Cyan test", "Yellow test"]) {
        const progressStyle = getComputedStyle(
          screen.getByRole("progressbar", { name: label }),
        );
        const boundary = progressStyle
          .getPropertyValue("--tray-progress-boundary-color")
          .trim();
        const track = progressStyle
          .getPropertyValue("--tray-progress-track-color")
          .trim();
        expect(boundary).not.toBe("");
        expect(track).not.toBe("");
        expect(
          contrastRatio(progressStyle.color, boundary),
        ).toBeGreaterThanOrEqual(3);
        expect(contrastRatio(boundary, track)).toBeGreaterThanOrEqual(3);
      }
    },
  );

  it("renders pending reset as a standalone state", () => {
    const snapshot = structuredClone(fullSnapshot);
    snapshot.agents[0].providers[0].subscription!.windows[0].resetsAt =
      "2026-07-16T11:59:00Z";
    snapshot.agents[0].providers[0].subscription!.windows[0].unavailableReason =
      "reset_pending_refresh";
    render(<TrayUsagePopoverView {...viewProps({ snapshot })} />);

    expect(screen.getAllByText("Pending refresh").length).toBeGreaterThan(0);
    expect(
      screen.queryByText("Resets Pending refresh"),
    ).not.toBeInTheDocument();
  });

  it("localizes allowlisted quota reasons and suppresses all raw reasons", () => {
    const snapshot = structuredClone(fullSnapshot);
    const windows = snapshot.agents[0].providers[0].subscription!.windows;
    windows[0].unavailableReason = "invalid_quota_percent";
    windows[1].unavailableReason = "oauth_token=must-never-render";
    render(<TrayUsagePopoverView {...viewProps({ snapshot })} />);

    expect(
      screen.getByText("Usage percentage unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByText("invalid_quota_percent")).not.toBeInTheDocument();
    expect(
      screen.queryByText("oauth_token=must-never-render"),
    ).not.toBeInTheDocument();
  });

  it("keeps footer labels visible without ellipsis in a compact wrapping layout", () => {
    render(<TrayUsagePopoverView {...viewProps()} />);

    const footer = screen.getByRole("contentinfo");
    const columns =
      getComputedStyle(footer).gridTemplateColumns.match(/minmax\(0, 1fr\)/g);
    expect(columns).toHaveLength(2);
    for (const name of ["Open details", "Refresh", "Settings", "Quit"]) {
      const label = screen.getByRole("button", { name }).querySelector("span");
      expect(label).not.toBeNull();
      const style = getComputedStyle(label!);
      expect(style.whiteSpace).toBe("normal");
      expect(style.overflow).not.toBe("hidden");
      expect(style.textOverflow).not.toBe("ellipsis");
    }
  });

  it("omits an empty subscription subtitle for an Agent without a plan", () => {
    const snapshot = structuredClone(fullSnapshot);
    snapshot.agents[0].providers[0].subscription!.planLabel = null;
    render(
      <TrayUsagePopoverView
        {...viewProps({ snapshot, selectedAgentId: "codex" })}
      />,
    );

    const article = screen.getByText("ChatGPT Plus/Pro").closest("article");
    expect(article).not.toBeNull();
    expect(article!.querySelector("p:empty")).toBeNull();
  });

  it("treats a billing-kind row without its matching payload as unavailable", () => {
    render(
      <TrayUsagePopoverView
        {...viewProps({
          snapshot: {
            ...fullSnapshot,
            status: "unknown",
            agents: [
              {
                agentModuleId: "broken-agent",
                name: "Broken Agent",
                sortOrder: 1,
                status: "unknown",
                providers: [
                  {
                    providerId: "broken-subscription",
                    providerName: "Broken Subscription",
                    systemPresetKey: null,
                    billingKind: "subscription",
                    status: "unknown",
                    warningReason: "raw_reason",
                    subscription: null,
                    metered: null,
                  },
                ],
              },
            ],
          },
        })}
      />,
    );

    expect(screen.getByText("No visible usage sources")).toBeInTheDocument();
    expect(screen.getAllByText("Data unavailable").length).toBeGreaterThan(0);
    expect(
      screen.queryByRole("region", { name: "Subscription" }),
    ).not.toBeInTheDocument();
  });
});

describe("TrayUsagePopover controller", () => {
  it("deduplicates the initial refresh under React Strict Mode", async () => {
    render(
      <StrictMode>
        <TrayUsagePopover />
      </StrictMode>,
      { wrapper: createQueryWrapper() },
    );

    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("refresh_tray_usage")).toHaveLength(1),
    );
  });

  it("refreshes once on mount, accepts events, and refreshes again on later shows", async () => {
    render(<TrayUsagePopover />, { wrapper: createQueryWrapper() });

    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("refresh_tray_usage")).toHaveLength(1),
    );
    expect(commandCalls("get_tray_usage_snapshot")).toHaveLength(1);

    act(() => {
      emitTauriEvent("tray-usage-updated", {
        ...fullSnapshot,
        status: "red",
      });
    });
    await waitFor(() =>
      expect(screen.getAllByText("Critical").length).toBeGreaterThan(0),
    );

    act(() => emitTauriEvent("tray-popover-shown"));
    await waitFor(() =>
      expect(commandCalls("refresh_tray_usage")).toHaveLength(2),
    );
  });

  it("hides at most once per show cycle and guards navigation blur", async () => {
    const user = userEvent.setup();
    render(<TrayUsagePopover />, { wrapper: createQueryWrapper() });
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.blur(window);
    await waitFor(() =>
      expect(commandCalls("hide_tray_popover")).toHaveLength(1),
    );

    act(() => emitTauriEvent("tray-popover-shown"));
    fireEvent.blur(window);
    await waitFor(() =>
      expect(commandCalls("hide_tray_popover")).toHaveLength(2),
    );

    act(() => emitTauriEvent("tray-popover-shown"));
    await user.click(screen.getByRole("button", { name: "Open details" }));
    expect(commandCalls("open_main_from_tray").at(-1)).toEqual([
      "open_main_from_tray",
      { destination: { kind: "usage", agentModuleId: null } },
    ]);
    fireEvent.blur(window);
    expect(commandCalls("hide_tray_popover")).toHaveLength(2);

    act(() => emitTauriEvent("tray-popover-shown"));
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(commandCalls("open_main_from_tray").at(-1)).toEqual([
      "open_main_from_tray",
      { destination: { kind: "providerBudget", providerId: null } },
    ]);
    fireEvent.blur(window);
    expect(commandCalls("hide_tray_popover")).toHaveLength(2);
  });

  it("allows only the first navigation destination in each show cycle", async () => {
    render(<TrayUsagePopover />, { wrapper: createQueryWrapper() });
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open details" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() =>
      expect(commandCalls("open_main_from_tray")).toHaveLength(1),
    );
    expect(commandCalls("open_main_from_tray")[0]).toEqual([
      "open_main_from_tray",
      { destination: { kind: "usage", agentModuleId: null } },
    ]);

    act(() => emitTauriEvent("tray-popover-shown"));
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(commandCalls("open_main_from_tray")).toHaveLength(1);
  });

  it("releases hide and navigation guards after command failures", async () => {
    const user = userEvent.setup();
    server.use(
      http.post("http://tauri.local/hide_tray_popover", () =>
        HttpResponse.json({ error: "hide failed" }, { status: 500 }),
      ),
    );
    render(<TrayUsagePopover />, { wrapper: createQueryWrapper() });
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    await settleLastCommand("hide_tray_popover");
    fireEvent.blur(window);
    await waitFor(() =>
      expect(commandCalls("hide_tray_popover")).toHaveLength(2),
    );

    server.use(
      http.post("http://tauri.local/open_main_from_tray", () =>
        HttpResponse.json({ error: "navigation failed" }, { status: 500 }),
      ),
    );
    act(() => emitTauriEvent("tray-popover-shown"));
    await user.click(screen.getByRole("button", { name: "Open details" }));
    await settleLastCommand("open_main_from_tray");
    fireEvent.blur(window);
    await waitFor(() =>
      expect(commandCalls("hide_tray_popover")).toHaveLength(3),
    );
  });
});
