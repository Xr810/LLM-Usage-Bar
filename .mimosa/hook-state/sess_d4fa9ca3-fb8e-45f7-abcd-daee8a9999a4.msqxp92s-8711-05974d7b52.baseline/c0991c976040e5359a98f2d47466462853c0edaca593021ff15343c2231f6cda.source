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

const openTab = async (name: string) => {
  const user = userEvent.setup();
  renderPage();
  await screen.findByText("ChatGPT");
  await user.click(screen.getByRole("tab", { name }));
  return user;
};

describe("usage breakdown dimensions", () => {
  it("keeps the model and agent dimensions idle until their tab is opened", async () => {
    renderPage();
    await screen.findByText("ChatGPT");

    expect(commandCalls("get_model_usage_dashboard")).toHaveLength(0);
    expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);
    expect(
      screen.getByRole("tab", { name: "Providers", selected: true }),
    ).toBeInTheDocument();
  });

  it("groups model usage by plan and drills into the models a plan served", async () => {
    const user = await openTab("Models");

    expect(
      await screen.findByRole("heading", { name: "By model" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("get_model_usage_dashboard")).toHaveLength(1),
    );

    const claude = await screen.findByTestId("model-group-claude-subscription");
    expect(within(claude).getByText("Claude Pro/Max")).toBeInTheDocument();
    expect(within(claude).getByText("$4.50")).toBeInTheDocument();
    // The plan's models stay collapsed until the row is opened.
    expect(within(claude).queryByText("claude-sonnet-5")).toBeNull();

    await user.click(within(claude).getByRole("button"));
    expect(within(claude).getByText("claude-sonnet-5")).toBeInTheDocument();
    expect(within(claude).getByText("claude-opus-5")).toBeInTheDocument();
  });

  it("merges one model across every account it ran on", async () => {
    const user = await openTab("Models");
    await screen.findByTestId("model-group-claude-subscription");

    await user.click(screen.getByRole("button", { name: "By model" }));

    const opus = await screen.findByTestId("model-total-claude-opus-5");
    expect(within(opus).getByText("claude-opus-5")).toBeInTheDocument();
    // Served by both the subscription and the metered account.
    expect(
      within(opus).getByText("Anthropic API · Claude Pro/Max"),
    ).toBeInTheDocument();
    expect(within(opus).getByText("$5.50")).toBeInTheDocument();
    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
      "17K",
    );
  });

  it("ranks agents and keeps unattributed usage in its own bucket", async () => {
    const user = await openTab("Agents");

    expect(
      await screen.findByRole("heading", { name: "By agent" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(1),
    );

    const rows = screen.getAllByTestId(/^agent-row-/);
    expect(rows.map((row) => row.dataset.testid)).toEqual([
      "agent-row-claude-code",
      "agent-row-codex",
      "agent-row-legacy-agent",
      "agent-row-unassigned",
    ]);
    expect(
      within(rows[2]).getByText("Archived", { selector: "*" }),
    ).toBeInTheDocument();
    expect(within(rows[3]).getAllByText("Unassigned").length).toBeGreaterThan(
      0,
    );

    await user.click(within(rows[1]).getByRole("button"));
    expect(within(rows[1]).getByText("Accounts")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Claude Pro/Max")).toBeInTheDocument();
  });

  it("shares one time range across every dimension", async () => {
    const user = await openTab("Models");
    await screen.findByTestId("model-group-claude-subscription");

    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_model_usage_dashboard").length,
      ).toBeGreaterThanOrEqual(2),
    );
    expect(screen.getByTestId("breakdown-range-label")).toHaveTextContent("7d");

    await user.click(screen.getByRole("tab", { name: "Agents" }));
    await waitFor(() =>
      expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(1),
    );

    const modelRange = commandCalls("get_model_usage_dashboard").at(-1)?.[1];
    const agentRange = commandCalls("get_agent_usage_breakdown").at(-1)?.[1];
    expect(agentRange?.startAt).toBe(modelRange?.startAt);
    expect(agentRange?.endAt).toBe(modelRange?.endAt);
  });
});
