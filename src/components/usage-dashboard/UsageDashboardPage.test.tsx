import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { UsageDashboardPage } from "./UsageDashboardPage";

const clients = new Set<QueryClient>();

afterEach(() => {
  // Unmount observers before cancelling queries and removing their GC timers.
  cleanup();
  for (const client of clients) client.clear();
  clients.clear();
  vi.restoreAllMocks();
});

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  clients.add(client);
  render(
    <QueryClientProvider client={client}>
      <UsageDashboardPage />
    </QueryClientProvider>,
  );
  return client;
}

describe("UsageDashboardPage Provider-only contract", () => {
  it("renders subscription and metered Provider accounts without Agent navigation", async () => {
    renderPage();

    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("ChatGPT")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Remaining quota" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Metered accounts" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).toHaveLength(2);
    expect(
      screen.getByRole("heading", { name: "Daily activity" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Usage trend" }),
    ).toBeInTheDocument();
    const trend = screen.getByRole("region", { name: "Usage trend" });
    const rangeControls = within(trend).getByRole("group", {
      name: /time range/i,
    });
    expect(
      within(rangeControls).getByRole("button", { name: "Today" }),
    ).toBeInTheDocument();
    expect(
      within(rangeControls).getByRole("button", { name: "1 year" }),
    ).toBeInTheDocument();
    expect(
      within(rangeControls).getByRole("button", { name: "Custom range" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Daily")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Token usage by day" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });

  it("queries exact Provider-wide ranges for today, 7 days, 30 days, and 1 year", async () => {
    // Fix only the range clock; user-event, MSW and query notifications use real timers.
    const now = new Date(2026, 6, 15, 12, 34, 56).getTime();
    vi.spyOn(Date, "now").mockReturnValue(now);
    const endAt = Math.floor(now / 1_000) + 1;
    const user = userEvent.setup();
    const client = renderPage();
    await screen.findByText("ChatGPT");
    expect(commandCalls("get_provider_usage_dashboard")[0]?.[1]).toEqual({
      startAt: Math.floor(new Date(2026, 5, 16).getTime() / 1_000),
      endAt,
    });

    const trend = within(screen.getByRole("region", { name: "Usage trend" }));
    // The activity heatmap has 365 day buttons unrelated to range selection.
    const controls = within(
      trend.getByRole("group", {
        name: /time range/i,
      }),
    );
    const ranges = [
      { name: "Today", startAt: new Date(2026, 6, 15), granularity: "hour" },
      { name: "7 days", startAt: new Date(2026, 6, 9), granularity: "day" },
      { name: "30 days", startAt: new Date(2026, 5, 16), granularity: "day" },
      { name: "1 year", startAt: new Date(2025, 6, 16), granularity: "day" },
    ];

    for (const [index, range] of ranges.entries()) {
      const startAt = Math.floor(range.startAt.getTime() / 1_000);
      await user.click(controls.getByRole("button", { name: range.name }));
      await waitFor(() => {
        const calls = commandCalls("get_provider_usage_dashboard");
        expect(calls).toHaveLength(index + 2);
        expect(calls.at(-1)?.[1]).toEqual({ startAt, endAt });
        // A recorded invoke is not a completed query. In particular the range
        // label changes before keepPreviousData has been replaced.
        const key = usageDashboardKeys.providerDashboard(startAt, endAt);
        expect(client.getQueryState(key)).toMatchObject({
          status: "success",
          fetchStatus: "idle",
        });
        expect(client.getQueryData(key)).toMatchObject({
          startAt,
          endAt,
          trendGranularity: range.granularity,
        });
      });
      expect(
        await trend.findByText(
          range.granularity === "hour" ? "Hourly" : "Daily",
        ),
      ).toBeInTheDocument();
      expect(
        controls.getByRole("button", { name: range.name }),
      ).toHaveAttribute("aria-pressed", "true");
    }
    expect(trend.getByTestId("usage-trend-range")).toHaveTextContent("1 year");

    const activityCalls = commandCalls("get_provider_usage_activity");
    expect(activityCalls).toHaveLength(1);
    expect(activityCalls[0]?.[1]).toEqual({
      startAt: Math.floor(new Date(2025, 6, 16).getTime() / 1_000),
      endAt: Math.floor(new Date(2026, 6, 16).getTime() / 1_000),
    });
  });

  it("does not expose manual Provider actions in the monitoring dashboard", async () => {
    renderPage();
    await screen.findByText("ChatGPT");

    expect(screen.queryByRole("button", { name: "Refresh quota" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Sync sessions" })).toBeNull();
    expect(commandCalls("refresh_provider_quota")).toHaveLength(0);
    expect(commandCalls("sync_provider_session_usage")).toHaveLength(0);
  });
});
