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
    expect(screen.getByRole("heading", { name: "Subscription accounts" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Metered Provider accounts" })).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).toHaveLength(2);
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });

  it("queries exact Provider-wide ranges for today, 7 days, and 30 days", async () => {
    const user = userEvent.setup();
    renderPage();
    await screen.findByText("ChatGPT Plus/Pro");

    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() =>
      expect(commandCalls("get_provider_usage_dashboard").length).toBeGreaterThan(1),
    );
    await user.click(screen.getByRole("button", { name: "30 days" }));
    await waitFor(() =>
      expect(commandCalls("get_provider_usage_dashboard").length).toBeGreaterThan(2),
    );

    for (const call of commandCalls("get_provider_usage_dashboard")) {
      const args = call[1] ?? {};
      expect(args).not.toHaveProperty("agentModuleId");
      expect(Number(args.endAt)).toBeGreaterThan(Number(args.startAt));
    }
  });

  it("refreshes quota and syncs sessions by Provider identity", async () => {
    const user = userEvent.setup();
    renderPage();
    await screen.findByText("ChatGPT Plus/Pro");

    await user.click(screen.getAllByRole("button", { name: "Refresh quota" })[0]);
    await waitFor(() =>
      expect(commandCalls("refresh_provider_quota").length).toBe(1),
    );
    expect(commandCalls("refresh_provider_quota")[0]?.[1]).toEqual({
      providerId: "system-chatgpt-subscription",
    });

    await user.click(screen.getAllByRole("button", { name: "Sync sessions" })[0]);
    await waitFor(() =>
      expect(commandCalls("sync_provider_session_usage").length).toBe(1),
    );
    expect(commandCalls("sync_provider_session_usage")[0]?.[1]).toEqual({
      providerId: "system-chatgpt-subscription",
    });
  });
});
