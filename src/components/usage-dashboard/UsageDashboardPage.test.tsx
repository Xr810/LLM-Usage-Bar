import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
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
    expect(screen.getByText("Daily")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Token usage by day" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });

  it("queries exact Provider-wide ranges for today, 7 days, and 30 days", async () => {
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

    for (const call of commandCalls("get_provider_usage_dashboard")) {
      const args = call[1] ?? {};
      expect(args).not.toHaveProperty("agentModuleId");
      expect(Number(args.endAt)).toBeGreaterThan(Number(args.startAt));
    }

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
