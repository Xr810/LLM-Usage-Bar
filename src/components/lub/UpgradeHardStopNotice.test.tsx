import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { SHOW_MONTHLY_REMINDER_NOTE } from "@/config/upgradeNotice";
import { TrayUsagePopover } from "@/components/tray-usage/TrayUsagePopover";
import { resetUpgradeNoticeDebug } from "@/lib/upgradeNoticeDebug";
import type { LicenseStatus } from "@/types/license";
import type { ConvertedBudget, UpgradeNotice } from "@/types";
import { server } from "../../../tests/msw/server";
import { commandCalls, emitTauriEvent } from "../../../tests/msw/tauriMocks";
import { designPanelFixture } from "./panelFixture";
import { TrayPanelV5 } from "./TrayPanelV5";
import {
  UpgradeHardStopNotice,
  type UpgradeHardStopNoticeProps,
} from "./UpgradeHardStopNotice";
import { useUpgradeNotice } from "./useUpgradeNotice";

const proLicense: LicenseStatus = {
  tier: "pro",
  state: "active",
  build: "official",
  features: [],
  uiFlags: { upsellEntry: false },
};

function sampleNotice(
  budgets: ConvertedBudget[] = sampleBudgets(),
  noticeId = "notice-1",
): UpgradeNotice {
  return {
    noticeId,
    convertedAt: "2026-10-29T08:50:00Z",
    budgets,
  };
}

function sampleBudgets(): ConvertedBudget[] {
  return [
    {
      budgetId: "shared",
      name: null,
      providerId: null,
      dailyCapUsd: "30.00",
      spentTodayUsd: "15.60",
      overToday: false,
    },
    {
      budgetId: "provider:anthropic",
      name: "Anthropic",
      providerId: "anthropic",
      dailyCapUsd: "10.00",
      spentTodayUsd: "12.40",
      overToday: true,
    },
    {
      budgetId: "provider:openai",
      name: "OpenAI",
      providerId: "openai",
      dailyCapUsd: "15.00",
      spentTodayUsd: "3.20",
      overToday: false,
    },
  ];
}

function renderNotice(overrides: Partial<UpgradeHardStopNoticeProps> = {}) {
  const props: UpgradeHardStopNoticeProps = {
    notice: sampleNotice(),
    license: proLicense,
    allowanceUsd: "5",
    onAck: vi.fn(async () => undefined),
    onSetHardStop: vi.fn(async () => ({ changed: true })),
    onAllowMoreToday: vi.fn(async () => undefined),
    onManageBudgets: vi.fn(),
    ...overrides,
  };
  const view = render(<UpgradeHardStopNotice {...props} />);
  return { ...view, props };
}

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { guard: en.guard }, true, true);
  i18n.addResourceBundle("zh", "translation", { guard: zh.guard }, true, true);
  await i18n.changeLanguage("en");
});

beforeEach(() => {
  resetUpgradeNoticeDebug();
});

afterEach(async () => {
  await i18n.changeLanguage("en");
});

describe("UpgradeHardStopNotice", () => {
  it("renders nothing for a null notice, a null license, community, or Free", () => {
    const { rerender, props } = renderNotice({ notice: null });
    expect(screen.queryByTestId("upgrade-hard-stop-notice")).toBeNull();

    rerender(
      <UpgradeHardStopNotice
        {...props}
        notice={sampleNotice()}
        license={null}
      />,
    );
    expect(screen.queryByTestId("upgrade-hard-stop-notice")).toBeNull();

    rerender(
      <UpgradeHardStopNotice
        {...props}
        notice={sampleNotice()}
        license={{ ...proLicense, build: "community", tier: "pro" }}
      />,
    );
    expect(screen.queryByTestId("upgrade-hard-stop-notice")).toBeNull();

    rerender(
      <UpgradeHardStopNotice
        {...props}
        notice={sampleNotice()}
        license={{ ...proLicense, tier: "free", state: "free" }}
      />,
    );
    expect(screen.queryByTestId("upgrade-hard-stop-notice")).toBeNull();

    rerender(
      <UpgradeHardStopNotice
        {...props}
        notice={sampleNotice()}
        license={{ ...proLicense, state: "free" }}
      />,
    );
    expect(screen.queryByTestId("upgrade-hard-stop-notice")).toBeNull();
  });

  it("splits over-today rows from the other converted budgets", () => {
    renderNotice();
    expect(
      screen.getByRole("region", { name: "Your budgets are now hard stops" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/your 3 reminder budgets now act as hard stops/),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Over today (1), the next request will be paused"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/·/)).toBeNull();
    expect(screen.getByText("Other converted budgets (2)")).toBeInTheDocument();
    expect(screen.queryByText(/Converted budgets \(/)).toBeNull();

    const over = document.querySelector(".lub-upgrade-over");
    expect(over).not.toBeNull();
    expect(
      within(over as HTMLElement).getByText("Anthropic"),
    ).toBeInTheDocument();
    expect(within(over as HTMLElement).getByText("$12.40")).toBeInTheDocument();
    expect(
      within(over as HTMLElement).getByText(/\/ \$10\.00\/day/),
    ).toBeInTheDocument();
    expect(
      within(over as HTMLElement).getByRole("button", {
        name: "Allow $5 more today",
      }),
    ).toBeInTheDocument();
    expect(
      within(over as HTMLElement).getByRole("button", {
        name: "Switch back to reminder",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /allow this one/i }),
    ).toBeNull();

    expect(screen.getByText("All providers (shared)")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("$15.60")).toBeInTheDocument();
    expect(screen.getByText("$3.20")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Got it" })).toHaveClass(
      "lub-btn-primary",
    );
    expect(
      screen.getByRole("button", { name: "Allow $5 more today" }),
    ).toHaveClass("lub-btn-primary");
  });

  it("uses the converted-budgets heading when nothing is over today", () => {
    const budgets = sampleBudgets().map((budget) => ({
      ...budget,
      overToday: false,
    }));
    renderNotice({ notice: sampleNotice(budgets) });
    expect(screen.queryByText(/next request will be paused/)).toBeNull();
    expect(document.querySelector(".lub-upgrade-over")).toBeNull();
    expect(screen.getByText("Converted budgets (3)")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Allow \$/ })).toBeNull();
    expect(
      screen.getAllByRole("button", { name: "Switch back to reminder" }),
    ).toHaveLength(3);
  });

  it("localizes a blank budget name the same way as null", () => {
    const blank = sampleBudgets().map((budget) => {
      if (budget.budgetId === "shared") return { ...budget, name: "" };
      if (budget.budgetId === "provider:openai")
        return { ...budget, name: "  " };
      return budget;
    });
    renderNotice({ notice: sampleNotice(blank) });
    expect(screen.getAllByText("All providers (shared)")).toHaveLength(2);
    expect(screen.queryByText("OpenAI")).toBeNull();
  });

  it("uses the English singular when one reminder budget was converted", () => {
    const budgets = [
      {
        budgetId: "provider:anthropic",
        name: "Anthropic",
        providerId: "anthropic",
        dailyCapUsd: "10.00",
        spentTodayUsd: "12.40",
        overToday: true,
      },
    ];
    renderNotice({ notice: sampleNotice(budgets), allowanceUsd: "20" });
    expect(
      screen.getByText(/your 1 reminder budget now acts as a hard stop/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/reminder budgets/)).toBeNull();
    expect(
      screen.getByRole("button", { name: "Allow $20 more today" }),
    ).toBeInTheDocument();
  });

  it("acks from Got it and from the close button, and manage budgets does not", async () => {
    const user = userEvent.setup();
    const first = renderNotice();
    await user.click(screen.getByRole("button", { name: "Got it" }));
    expect(first.props.onAck).toHaveBeenCalledWith("notice-1");
    expect(first.props.onManageBudgets).not.toHaveBeenCalled();
    first.unmount();

    const second = renderNotice();
    await user.click(screen.getByRole("button", { name: "Dismiss notice" }));
    expect(second.props.onAck).toHaveBeenCalledWith("notice-1");

    await user.click(screen.getByRole("button", { name: "Manage budgets" }));
    expect(second.props.onManageBudgets).toHaveBeenCalledOnce();
    expect(second.props.onAck).toHaveBeenCalledTimes(1);
  });

  it("calls allow-more with the row action and leaves the notice open", async () => {
    const user = userEvent.setup();
    const { props } = renderNotice();
    await user.click(
      screen.getByRole("button", { name: "Allow $5 more today" }),
    );
    expect(props.onAllowMoreToday).toHaveBeenCalledOnce();
    expect(props.onAck).not.toHaveBeenCalled();
    expect(screen.getByTestId("upgrade-hard-stop-notice")).toBeInTheDocument();
  });

  it("names the budgets that failed to switch back and retries only those", async () => {
    const user = userEvent.setup();
    const onSetHardStop = vi.fn(async (budgetId: string) => {
      if (budgetId === "provider:openai") throw new Error("openai failed");
      return { changed: true };
    });
    renderNotice({ onSetHardStop });

    await user.click(
      screen.getByRole("button", { name: "Switch all back to reminders" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't switch 1 back: OpenAI. Try again.",
    );
    expect(onSetHardStop).toHaveBeenCalledTimes(3);
    expect(screen.getAllByText("Back to reminder")).toHaveLength(2);
    expect(
      screen.getAllByRole("button", { name: "Switch back to reminder" }),
    ).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: "Allow $5 more today" }),
    ).toBeNull();

    onSetHardStop.mockClear();
    onSetHardStop.mockResolvedValue({ changed: true });
    await user.click(
      screen.getByRole("button", { name: "Switch all back to reminders" }),
    );
    await waitFor(() => expect(onSetHardStop).toHaveBeenCalledTimes(1));
    expect(onSetHardStop).toHaveBeenCalledWith("provider:openai");
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(screen.getAllByText("Back to reminder")).toHaveLength(3);
  });

  it("keeps the monthly reminder line behind the config flag", () => {
    expect(SHOW_MONTHLY_REMINDER_NOTE).toBe(true);
    const { rerender, props } = renderNotice();
    expect(
      screen.getByText(
        "Monthly budgets still only remind you; they never block.",
      ),
    ).toBeInTheDocument();
    rerender(<UpgradeHardStopNotice {...props} showMonthlyNote={false} />);
    expect(
      screen.queryByText(
        "Monthly budgets still only remind you; they never block.",
      ),
    ).toBeNull();
    expect(
      screen.getByText(/your 3 reminder budgets now act as hard stops/),
    ).toBeInTheDocument();
  });

  it("uses the approved Chinese copy, including the comma in the over-today heading", async () => {
    await i18n.changeLanguage("zh");
    renderNotice();
    expect(
      screen.getByRole("region", { name: "预算已改为硬停" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/你原有的 3 条提醒预算已改为硬停/),
    ).toBeInTheDocument();
    expect(screen.getByText("月预算仍只提醒，不会拦截。")).toBeInTheDocument();
    expect(
      screen.getByText("今天已超限（1），下一个请求会被暂停"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/·/)).toBeNull();
    expect(screen.getByText("全部服务商合计")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "今天多放行 $5" }),
    ).toBeInTheDocument();
  });

  it("places the card at the top of the tray panel", () => {
    render(
      <TrayPanelV5
        model={designPanelFixture()}
        locale="en"
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
        upgradeNotice={
          <UpgradeHardStopNotice
            notice={sampleNotice()}
            license={proLicense}
            onAck={vi.fn()}
            onSetHardStop={vi.fn(async () => ({ changed: true }))}
            onAllowMoreToday={vi.fn()}
            onManageBudgets={vi.fn()}
          />
        }
      />,
    );
    const panel = screen.getByTestId("lub-tray-panel");
    expect(panel.firstElementChild).toHaveAttribute(
      "data-testid",
      "upgrade-hard-stop-notice",
    );
  });
});

function Harness() {
  const state = useUpgradeNotice();
  return (
    <div>
      <span data-testid="notice-id">{state.notice?.noticeId ?? "null"}</span>
      <span data-testid="allowance">{state.allowanceUsd}</span>
      <button
        type="button"
        onClick={() => void state.ack(state.notice?.noticeId ?? "notice-1")}
      >
        ack
      </button>
      <button type="button" onClick={() => void state.allowMoreToday()}>
        allow
      </button>
      <button
        type="button"
        onClick={() => void state.setHardStop("provider:openai")}
      >
        switch
      </button>
    </div>
  );
}

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("useUpgradeNotice", () => {
  it("degrades a missing command to null and logs once at debug", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
    server.use(
      http.post(
        "http://tauri.local/guard_get_upgrade_notice",
        () => new HttpResponse("missing command", { status: 404 }),
      ),
    );
    render(<Harness />);
    expect(await screen.findByTestId("notice-id")).toHaveTextContent("null");
    await waitFor(() =>
      expect(debugSpy).toHaveBeenCalledWith(
        "[upgrade-notice]",
        expect.stringContaining("missing command"),
      ),
    );
    expect(
      errorSpy.mock.calls.filter((args) =>
        args.some((arg) => String(arg).includes("upgrade-notice")),
      ),
    ).toEqual([]);
    const before = debugSpy.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "ack" }));
    actEmit("guard-state-changed");
    await waitFor(() =>
      expect(commandCalls("guard_get_upgrade_notice").length).toBeGreaterThan(
        1,
      ),
    );
    expect(debugSpy.mock.calls.length).toBe(before);
    errorSpy.mockRestore();
    debugSpy.mockRestore();
  });

  it("treats a malformed payload as no notice", async () => {
    const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json({ nope: true }),
      ),
    );
    render(<Harness />);
    expect(await screen.findByTestId("notice-id")).toHaveTextContent("null");
    await waitFor(() =>
      expect(debugSpy).toHaveBeenCalledWith(
        "[upgrade-notice]",
        expect.stringContaining("unexpected payload"),
      ),
    );
    debugSpy.mockRestore();
  });

  it("fetches on startup, license-changed, and guard-state-changed", async () => {
    let calls = 0;
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () => {
        calls += 1;
        if (calls === 1) return HttpResponse.json(null);
        if (calls === 2) return HttpResponse.json(sampleNotice());
        return HttpResponse.json(sampleNotice(sampleBudgets(), "notice-2"));
      }),
    );
    render(<Harness />);
    expect(await screen.findByTestId("notice-id")).toHaveTextContent("null");
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(1);

    actEmit("license-changed");
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(2);

    actEmit("guard-state-changed");
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-2"),
    );
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(3);
  });

  it("acks by notice id and calls allow-more with no amount", async () => {
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json(sampleNotice()),
      ),
      http.post("http://tauri.local/get_ui_prefs", () =>
        HttpResponse.json({
          guard: { allowMoreTodayDefaultUsd: "10" },
        }),
      ),
    );
    render(<Harness />);
    expect(await screen.findByTestId("notice-id")).toHaveTextContent(
      "notice-1",
    );
    await waitFor(() =>
      expect(screen.getByTestId("allowance")).toHaveTextContent("10"),
    );

    fireEvent.click(screen.getByRole("button", { name: "ack" }));
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("null"),
    );
    expect(commandCalls("guard_ack_upgrade_notice")[0]?.[1]).toEqual({
      noticeId: "notice-1",
    });

    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    await waitFor(() =>
      expect(commandCalls("guard_allow_more_today")).toHaveLength(1),
    );
    expect(commandCalls("guard_allow_more_today")[0]).toHaveLength(1);
    expect(commandCalls("guard_allow_more_today")[0]?.[1]).toBeUndefined();

    fireEvent.click(screen.getByRole("button", { name: "switch" }));
    await waitFor(() =>
      expect(commandCalls("guard_set_budget_hard_stop")).toHaveLength(1),
    );
    expect(commandCalls("guard_set_budget_hard_stop")[0]?.[1]).toEqual({
      budgetId: "provider:openai",
      hardStop: false,
    });
  });

  it("shows the notice for Pro at the top of the live tray and navigates without acking", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json(proLicense),
      ),
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json(sampleNotice()),
      ),
    );
    render(<TrayUsagePopover />, { wrapper: wrapper() });
    await waitFor(() => {
      const panel = screen.getByTestId("lub-tray-panel");
      expect(panel.firstElementChild).toHaveAttribute(
        "data-testid",
        "upgrade-hard-stop-notice",
      );
    });
    expect(
      screen.getByRole("button", { name: "Allow $5 more today" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Manage budgets" }));
    await waitFor(() =>
      expect(commandCalls("open_main_from_tray")).toHaveLength(1),
    );
    expect(commandCalls("open_main_from_tray")[0]?.[1]).toEqual({
      destination: { kind: "providerBudget", providerId: null },
    });
    expect(commandCalls("guard_ack_upgrade_notice")).toHaveLength(0);

    fireEvent.click(
      screen.getByRole("button", { name: "Allow $5 more today" }),
    );
    await waitFor(() =>
      expect(commandCalls("guard_allow_more_today")).toHaveLength(1),
    );
    expect(commandCalls("guard_allow_more_today")[0]?.[1]).toBeUndefined();
  });

  it("re-fetches the notice after a hard-stop that changed, without a guard event", async () => {
    let gets = 0;
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () => {
        gets += 1;
        return HttpResponse.json(
          gets === 1
            ? sampleNotice()
            : sampleNotice(sampleBudgets(), "notice-after-stop"),
        );
      }),
      http.post("http://tauri.local/guard_set_budget_hard_stop", () =>
        HttpResponse.json({ changed: true }),
      ),
    );
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    const before = commandCalls("guard_get_upgrade_notice").length;

    fireEvent.click(screen.getByRole("button", { name: "switch" }));
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent(
        "notice-after-stop",
      ),
    );
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before + 1);
    expect(commandCalls("guard_set_budget_hard_stop")).toHaveLength(1);
  });

  it("does not re-fetch when a hard-stop reports changed false", async () => {
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json(sampleNotice()),
      ),
      http.post("http://tauri.local/guard_set_budget_hard_stop", () =>
        HttpResponse.json({ changed: false }),
      ),
    );
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    const before = commandCalls("guard_get_upgrade_notice").length;

    fireEvent.click(screen.getByRole("button", { name: "switch" }));
    await waitFor(() =>
      expect(commandCalls("guard_set_budget_hard_stop")).toHaveLength(1),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before);
    expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1");
  });

  it("hides on ack before the command resolves and re-fetches only when changed", async () => {
    let releaseAck: (result: { changed: boolean }) => void = () => {};
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json(sampleNotice()),
      ),
      http.post("http://tauri.local/guard_ack_upgrade_notice", async () => {
        const result = await new Promise<{ changed: boolean }>((resolve) => {
          releaseAck = resolve;
        });
        return HttpResponse.json(result);
      }),
    );
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    const before = commandCalls("guard_get_upgrade_notice").length;

    fireEvent.click(screen.getByRole("button", { name: "ack" }));
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("null"),
    );
    expect(commandCalls("guard_ack_upgrade_notice")).toHaveLength(1);
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before);

    await act(async () => {
      releaseAck({ changed: true });
    });
    await waitFor(() =>
      expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before + 1),
    );
    expect(screen.getByTestId("notice-id")).toHaveTextContent("null");
  });

  it("hides on ack without re-fetching when changed is false", async () => {
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () =>
        HttpResponse.json(sampleNotice()),
      ),
      http.post("http://tauri.local/guard_ack_upgrade_notice", () =>
        HttpResponse.json({ changed: false }),
      ),
    );
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    const before = commandCalls("guard_get_upgrade_notice").length;

    fireEvent.click(screen.getByRole("button", { name: "ack" }));
    await waitFor(() =>
      expect(commandCalls("guard_ack_upgrade_notice")).toHaveLength(1),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByTestId("notice-id")).toHaveTextContent("null");
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before);
  });

  it("re-fetches the notice after allow-more without a guard event", async () => {
    let gets = 0;
    server.use(
      http.post("http://tauri.local/guard_get_upgrade_notice", () => {
        gets += 1;
        return HttpResponse.json(
          gets === 1
            ? sampleNotice()
            : sampleNotice(sampleBudgets(), "notice-after-allow"),
        );
      }),
    );
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent("notice-1"),
    );
    const before = commandCalls("guard_get_upgrade_notice").length;

    fireEvent.click(screen.getByRole("button", { name: "allow" }));
    await waitFor(() =>
      expect(screen.getByTestId("notice-id")).toHaveTextContent(
        "notice-after-allow",
      ),
    );
    expect(commandCalls("guard_allow_more_today")).toHaveLength(1);
    expect(commandCalls("guard_allow_more_today")[0]?.[1]).toBeUndefined();
    expect(commandCalls("guard_get_upgrade_notice")).toHaveLength(before + 1);
  });
});

function actEmit(event: string) {
  act(() => {
    emitTauriEvent(event);
  });
}
