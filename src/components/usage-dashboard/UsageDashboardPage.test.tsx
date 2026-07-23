import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { UsageDashboardPage } from "./UsageDashboardPage";

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <UsageDashboardPage />
    </QueryClientProvider>,
  );
}

describe("UsageDashboardPage Provider-only contract", () => {
  it("renders subscription and metered Provider accounts without Agent navigation", async () => {
    renderPage();

    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();
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
    const user = userEvent.setup();
    renderPage();
    await screen.findByText("ChatGPT Plus/Pro");

    await user.click(screen.getByRole("button", { name: "Today" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(1),
    );
    expect(await screen.findByText("Hourly")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(2),
    );
    expect(await screen.findByText("Daily")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "30 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(3),
    );
    await user.click(screen.getByRole("button", { name: "1 year" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(4),
    );
    expect(screen.getByTestId("usage-trend-range")).toHaveTextContent("1 year");

    for (const call of commandCalls("get_provider_usage_dashboard")) {
      const args = call[1] ?? {};
      expect(args).not.toHaveProperty("agentModuleId");
      expect(Number(args.endAt)).toBeGreaterThan(Number(args.startAt));
    }

    const oneYearCall = commandCalls("get_provider_usage_dashboard").at(-1);
    const oneYearArgs = oneYearCall?.[1] ?? {};
    const oneYearDuration =
      Number(oneYearArgs.endAt) - Number(oneYearArgs.startAt);
    expect(oneYearDuration).toBeGreaterThan(364 * 24 * 60 * 60);
    expect(oneYearDuration).toBeLessThan(366 * 24 * 60 * 60);

    const activityCalls = commandCalls("get_provider_usage_activity");
    expect(activityCalls).toHaveLength(1);
    const activityArgs = activityCalls[0]?.[1] ?? {};
    expect(activityArgs).not.toHaveProperty("agentModuleId");
    expect(
      Number(activityArgs.endAt) - Number(activityArgs.startAt),
    ).toBeGreaterThan(300 * 24 * 60 * 60);
  });

  it("does not expose manual Provider actions in the monitoring sidebar", async () => {
    renderPage();
    await screen.findByText("ChatGPT Plus/Pro");

    expect(
      screen.queryByRole("button", { name: "Refresh quota" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Sync sessions" }),
    ).toBeNull();
    expect(commandCalls("refresh_provider_quota")).toHaveLength(0);
    expect(commandCalls("sync_provider_session_usage")).toHaveLength(0);
  });
});
