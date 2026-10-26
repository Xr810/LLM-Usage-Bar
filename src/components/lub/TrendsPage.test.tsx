import type { ReactElement } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import { HEATMAP_MAX_WEEKS } from "@/lib/constants";
import { server } from "../../../tests/msw/server";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { TrendsPage } from "./TrendsPage";
import { WEEKDAY_HOUR_FEATURE } from "./trendsModel";

beforeEach(async () => {
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

    expect(await screen.findByText("OpenAI API")).toBeInTheDocument();
    expect(screen.getByText("Anthropic API")).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter").length).toBeGreaterThan(0);
    expect(screen.queryByText("Claude Team")).toBeNull();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.queryByText("$159.04")).toBeNull();
    expect(screen.getByTestId("lub-trend-total")).toHaveTextContent(
      formatUsd("0.125000", "zh"),
    );
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

  it("keeps week, month, project, and client behind upsell and opens one panel", async () => {
    const user = userEvent.setup();
    renderPage(<TrendsPage features={["upsell_entry"]} />);
    expect(screen.getByTestId("lub-weekday-hour")).toBeInTheDocument();
    expect(screen.getAllByText("PRO").length).toBeGreaterThan(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();

    await user.click(screen.getByRole("button", { name: "周，Pro 功能" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("用 Pro 看更多维度");
    expect(within(dialog).getByText("比较 Free 与 Pro")).toBeInTheDocument();
    expect(within(dialog).getByText("开始 7 天免费试用")).toBeInTheDocument();
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

  it("lets Pro switch week and group by project or client", async () => {
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
    expect(
      within(toolbar).getByRole("button", { name: "项目" }),
    ).toBeInTheDocument();
    expect(
      within(toolbar).getByRole("button", { name: "客户" }),
    ).toBeInTheDocument();

    await screen.findByText("OpenAI API");
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

    await user.click(within(toolbar).getByRole("button", { name: "项目" }));
    expect(await screen.findByText("Claude Code")).toBeInTheDocument();
    expect(screen.queryByText("OpenAI API")).toBeNull();
    expect(screen.queryByText("Retired Agent")).toBeNull();
    expect(commandCalls("get_agent_usage_breakdown").length).toBeGreaterThan(0);

    await user.click(within(toolbar).getByRole("button", { name: "客户" }));
    expect(await screen.findByText("codex")).toBeInTheDocument();
    expect(screen.getByText("claude")).toBeInTheDocument();
    expect(screen.queryByText("Claude Code")).toBeNull();
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
