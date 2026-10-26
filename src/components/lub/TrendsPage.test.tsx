import type { ReactElement } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import { HEATMAP_MAX_WEEKS } from "@/lib/constants";
import { server } from "../../../tests/msw/server";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { TrendsPage } from "./TrendsPage";
import {
  QUERY_USAGE_TREND_COMMAND,
  type PerGroupDailySeries,
} from "./perGroupDailyTrend";
import { WEEKDAY_HOUR_FEATURE, type TrendDimension } from "./trendsModel";

const perGroupDaily = vi.hoisted(() => ({
  impl: null as
    ((groupBy: TrendDimension) => PerGroupDailySeries | null) | null,
}));

vi.mock("./perGroupDailyTrend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./perGroupDailyTrend")>();
  return {
    ...actual,
    loadPerGroupDailySeries: (groupBy: TrendDimension) =>
      perGroupDaily.impl
        ? perGroupDaily.impl(groupBy)
        : actual.loadPerGroupDailySeries(groupBy),
  };
});

beforeEach(async () => {
  perGroupDaily.impl = null;
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("zh");
});

function renderPage(ui: ReactElement = <TrendsPage />) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>{ui}</QueryClientProvider>,
  );
}

function buttonNamed(name: string) {
  return screen.queryByRole("button", {
    name: (accessible) => {
      const text = accessible.replace(/\s+/g, " ").trim();
      return text === name || text.startsWith(`${name} `);
    },
  });
}

describe("TrendsPage", () => {
  it("shows a free day view and a 52-week heatmap with no upgrade entry", async () => {
    renderPage();
    expect(HEATMAP_MAX_WEEKS).toBe(52);
    expect(screen.getByRole("navigation")).toHaveAccessibleName("设置");
    expect(screen.getByRole("navigation")).not.toHaveAccessibleName("日");
    expect(
      screen.getByRole("heading", { name: "趋势与热力图" }),
    ).toBeInTheDocument();
    expect(screen.getByText("近 30 天")).toBeInTheDocument();
    expect(screen.getByText("近 52 周每日花费")).toBeInTheDocument();
    expect(screen.queryByTestId("lub-trends-toolbar")).toBeNull();
    expect(buttonNamed("周")).toBeNull();
    expect(buttonNamed("月")).toBeNull();
    expect(buttonNamed("项目")).toBeNull();
    expect(buttonNamed("客户")).toBeNull();
    expect(screen.queryByText("PRO")).toBeNull();
    expect(screen.queryByText("星期与时段")).toBeNull();
    expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();
    expect(screen.queryByText("开始 7 天免费试用")).toBeNull();
    expect(
      screen.queryByRole("group", { name: /time range|时间范围/i }),
    ).toBeNull();
    expect(
      screen.getByTestId("lub-trends").querySelector("a, [href]"),
    ).toBeNull();
    expect(document.querySelectorAll(".lub-heat-grid i")).toHaveLength(52 * 7);
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-week-start",
      "1",
    );

    const totalText = formatUsd("0.125000", "zh");
    await waitFor(() => {
      expect(screen.getByTestId("lub-trend-total")).toHaveTextContent(
        totalText,
      );
    });
    expect(screen.queryByText("OpenAI API")).toBeNull();
    expect(screen.queryByText("Anthropic API")).toBeNull();
    expect(screen.queryByText("OpenRouter")).toBeNull();
    expect(screen.queryByText("Claude Code")).toBeNull();
    expect(screen.queryByText("codex")).toBeNull();
    expect(screen.queryByText("claude")).toBeNull();
    const bars = document.querySelectorAll(".lub-trends-chart > span");
    expect(bars.length).toBeGreaterThan(0);
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-per-group",
      "none",
    );
    for (const bar of bars) {
      expect(bar).toHaveAttribute("data-series", "total");
      expect(bar.querySelectorAll("i").length).toBeLessThanOrEqual(1);
      expect(bar.querySelector("[data-slot='1']")).toBeNull();
    }
    expect(screen.getByTestId("lub-trend-line-baseline")).toBeInTheDocument();
    expect(screen.getByTestId("lub-trend-line-max")).toHaveTextContent(
      formatUsd("0.125000", "zh"),
    );
    expect(screen.queryByTestId("lub-trend-line-cap")).toBeNull();
    const axis = screen.getByTestId("lub-trends-axis");
    expect(axis.querySelectorAll("time").length).toBeGreaterThan(7);
    expect(axis.querySelector("time")?.getAttribute("datetime")).toMatch(
      /^\d{4}-\d{2}-\d{2}$/,
    );

    await waitFor(() => {
      const hot = document.querySelectorAll(
        '.lub-heat-grid i[data-level="1"], .lub-heat-grid i[data-level="2"], .lub-heat-grid i[data-level="3"], .lub-heat-grid i[data-level="4"]',
      );
      expect(hot.length).toBe(1);
    });

    const dashboard = commandCalls("get_provider_usage_dashboard").at(-1)?.[1];
    const dashSpan = Number(dashboard?.endAt) - Number(dashboard?.startAt);
    expect(dashSpan).toBeGreaterThan(29 * 86_400);
    expect(dashSpan).toBeLessThan(30 * 86_400 + 5);
    const activity = commandCalls("get_provider_usage_activity").at(-1)?.[1];
    const weeks =
      (Number(activity?.endAt) - Number(activity?.startAt)) / (7 * 86_400);
    expect(weeks).toBeGreaterThan(51.5);
    expect(weeks).toBeLessThan(52.5);
    expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);
    expect(commandCalls("get_usage_trends")).toHaveLength(0);
    expect(commandCalls(QUERY_USAGE_TREND_COMMAND)).toHaveLength(0);
  });

  it("draws the shared daily cap as a reference line above the peak", async () => {
    server.use(
      http.post("http://tauri.local/get_api_budget_config", () =>
        HttpResponse.json({ mode: "shared", sharedDailyBudgetUsd: "10" }),
      ),
    );
    renderPage();
    const cap = await screen.findByTestId("lub-trend-line-cap");
    const max = screen.getByTestId("lub-trend-line-max");
    expect(cap).toHaveTextContent(formatUsd("10.000000", "zh"));
    expect(Number.parseFloat(cap.style.bottom)).toBeGreaterThan(
      Number.parseFloat(max.style.bottom),
    );
  });

  it("keeps week and month behind upsell and hides project and client", async () => {
    const user = userEvent.setup();
    renderPage(<TrendsPage features={["upsell_entry"]} />);
    expect(screen.getByTestId("lub-weekday-hour")).toBeInTheDocument();
    expect(screen.getAllByText("PRO").length).toBeGreaterThan(0);
    expect(buttonNamed("项目")).toBeNull();
    expect(buttonNamed("客户")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();

    await user.click(screen.getByRole("button", { name: "周，Pro 功能" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent(zh.lub.procompare.title);
    expect(
      within(dialog).getByRole("table", { name: zh.lub.upgrade.aria_table }),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(zh.lub.procompare.group_stop),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByText(zh.lub.procompare.start_trial),
    ).toBeNull();
    expect(within(dialog).queryByText(zh.lub.procompare.trial)).toBeNull();
    expect(within(dialog).queryByText(zh.lub.procompare.beta_free)).toBeNull();
    expect(within(dialog).queryByText(zh.lub.purchase.buy)).toBeNull();
    expect(dialog.querySelector("[data-slot='purchase']")).toHaveTextContent(
      "",
    );
    expect(dialog.querySelector("a, [href]")).toBeNull();
    expect(screen.getByRole("button", { name: "日" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      screen.queryByRole("group", { name: /time range|时间范围/i }),
    ).toBeNull();

    await user.click(screen.getByRole("button", { name: "以后再说" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the weekday card for its own feature without a pro tag", () => {
    renderPage(<TrendsPage features={[WEEKDAY_HOUR_FEATURE]} />);
    expect(screen.getByTestId("lub-weekday-hour")).toBeInTheDocument();
    expect(screen.queryByText("PRO")).toBeNull();
    expect(screen.queryByTestId("lub-trends-toolbar")).toBeNull();
  });

  it("lets Pro switch week while project and client stay hidden", async () => {
    const user = userEvent.setup();
    renderPage(<TrendsPage tier="pro" />);
    expect(screen.queryByText("PRO")).toBeNull();
    expect(screen.queryByTestId("lub-weekday-hour")).toBeNull();
    expect(
      screen.queryByRole("group", { name: /time range|时间范围/i }),
    ).toBeNull();
    const toolbar = screen.getByTestId("lub-trends-toolbar");
    expect(
      within(toolbar).getByRole("button", { name: "周" }),
    ).toBeInTheDocument();
    expect(
      within(toolbar).getByRole("button", { name: "月" }),
    ).toBeInTheDocument();
    expect(within(toolbar).queryByRole("button", { name: "项目" })).toBeNull();
    expect(within(toolbar).queryByRole("button", { name: "客户" })).toBeNull();

    await waitFor(() => {
      expect(screen.getByTestId("lub-trend-total")).not.toHaveTextContent("—");
    });
    const dayLabels = screen
      .getByTestId("lub-trends-axis")
      .querySelectorAll("time").length;
    expect(dayLabels).toBeGreaterThan(20);
    await user.click(within(toolbar).getByRole("button", { name: "周" }));
    expect(within(toolbar).getByRole("button", { name: "周" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await waitFor(() => {
      expect(
        screen.getByTestId("lub-trends-axis").querySelectorAll("time").length,
      ).toBeLessThan(dayLabels);
    });

    expect(screen.queryByText("Claude Code")).toBeNull();
    expect(screen.queryByText("OpenAI API")).toBeNull();
    expect(screen.queryByText("codex")).toBeNull();
    expect(screen.queryByText("claude")).toBeNull();
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-per-group",
      "none",
    );
    expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);
    expect(commandCalls(QUERY_USAGE_TREND_COMMAND)).toHaveLength(0);
  });

  it("shows a project toggle only after that dimension's series exists", async () => {
    const user = userEvent.setup();
    perGroupDaily.impl = (groupBy) =>
      groupBy === "project" ? { groupBy } : null;
    renderPage(<TrendsPage tier="pro" />);
    const toolbar = screen.getByTestId("lub-trends-toolbar");
    expect(within(toolbar).queryByRole("button", { name: "客户" })).toBeNull();
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-per-group",
      "none",
    );
    await user.click(within(toolbar).getByRole("button", { name: "项目" }));
    expect(
      within(toolbar).getByRole("button", { name: "项目" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-per-group",
      "day",
    );
    expect(commandCalls(QUERY_USAGE_TREND_COMMAND)).toHaveLength(0);
  });

  it("lets Pro switch project and client when both series exist", async () => {
    const user = userEvent.setup();
    perGroupDaily.impl = (groupBy) =>
      groupBy === "provider" ? null : { groupBy };
    renderPage(<TrendsPage tier="pro" />);
    const toolbar = screen.getByTestId("lub-trends-toolbar");
    await user.click(within(toolbar).getByRole("button", { name: "客户" }));
    expect(
      within(toolbar).getByRole("button", { name: "客户" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("lub-trends")).toHaveAttribute(
      "data-per-group",
      "day",
    );
    await user.click(within(toolbar).getByRole("button", { name: "项目" }));
    expect(
      within(toolbar).getByRole("button", { name: "项目" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText("PRO")).toBeNull();
    expect(commandCalls(QUERY_USAGE_TREND_COMMAND)).toHaveLength(0);
  });

  it("locks project and client behind upsell when their series exist", async () => {
    const user = userEvent.setup();
    perGroupDaily.impl = (groupBy) =>
      groupBy === "provider" ? null : { groupBy };
    renderPage(<TrendsPage features={["upsell_entry"]} />);
    const project = screen.getByRole("button", { name: "项目，Pro 功能" });
    expect(
      screen.getByRole("button", { name: "客户，Pro 功能" }),
    ).toBeInTheDocument();
    await user.click(project);
    expect(screen.getByRole("dialog")).toHaveTextContent(
      zh.lub.procompare.title,
    );
    expect(screen.queryByText(zh.lub.procompare.start_trial)).toBeNull();
    expect(screen.queryByText(zh.lub.purchase.buy)).toBeNull();
    expect(screen.getByRole("button", { name: "日" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(project).toHaveAttribute("aria-pressed", "false");
    expect(commandCalls(QUERY_USAGE_TREND_COMMAND)).toHaveLength(0);
  });

  it("uses the English last-30-days and heatmap keys", async () => {
    await i18n.changeLanguage("en");
    renderPage();
    expect(screen.getByText("Last 30 days")).toBeInTheDocument();
    expect(screen.getByText("Daily spend, last 52 weeks")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Trends & heatmaps" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("navigation")).toHaveAccessibleName("Settings");
    expect(screen.queryByText("近 30 天")).toBeNull();
    expect(screen.queryByTestId("lub-trends-toolbar")).toBeNull();
  });
});
