import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { usageDashboardApi } from "@/lib/api/usageDashboard";

const renderSettings = (open = true, defaultTab?: string) => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <SettingsPage open={open} onOpenChange={() => {}} defaultTab={defaultTab} />
    </QueryClientProvider>,
  );
  return { ...view, client };
};

describe("SettingsPage Provider-only integration", () => {
  it("loads Provider accounts and diagnostics without Agent or Proxy sections", async () => {
    const user = userEvent.setup();
    renderSettings();

    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).not.toHaveLength(0);
    expect(screen.queryByRole("tab", { name: "Agents" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Proxy setup" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText("legacy-provider")).toBeInTheDocument();
    expect(screen.getByText("custom-archived")).toBeInTheDocument();
    expect(screen.getByText("cross_agent")).toBeInTheDocument();
  });

  it("renders immutable system Provider cards and never retains an upstream key", async () => {
    const user = userEvent.setup();
    const view = renderSettings(true, "providers");

    const systemCards = await screen.findAllByTestId(/^system-provider-/);
    expect(
      systemCards.map(
        (card) => within(card).getByRole("heading", { level: 3 }).textContent,
      ),
    ).toEqual([
      "ChatGPT Plus/Pro",
      "Claude Pro/Max",
      "OpenAI API",
      "Anthropic API",
      "OpenRouter",
    ]);
    for (const card of systemCards) {
      expect(within(card).queryByRole("button", { name: /edit/i })).toBeNull();
      expect(
        within(card).queryByRole("button", { name: /delete/i }),
      ).toBeNull();
      expect(within(card).queryByText("Agent bindings")).toBeNull();
    }

    const openAiCard = screen.getByTestId("system-provider-system-openai-api");
    const upstreamKey = "transient-upstream-provider-sentinel";
    await user.type(within(openAiCard).getByLabelText("API key"), upstreamKey);
    await user.click(
      within(openAiCard).getByRole("button", { name: "Verify" }),
    );
    await waitFor(() =>
      expect(
        within(openAiCard).getByText("Connection succeeded"),
      ).toBeInTheDocument(),
    );

    const frontendSnapshot = JSON.stringify({
      dom: view.baseElement.innerHTML,
      formValues: Array.from(
        view.baseElement.querySelectorAll<HTMLInputElement>("input"),
        (element) => element.value,
      ),
      queries: view.client
        .getQueryCache()
        .getAll()
        .map((query) => query.state.data),
      mutations: view.client
        .getMutationCache()
        .getAll()
        .map((mutation) => mutation.state.data),
    });
    expect(frontendSnapshot).not.toContain(upstreamKey);
  });

  it("edits daily budgets only on metered Provider accounts", async () => {
    const user = userEvent.setup();
    renderSettings(true, "providers");

    const chatGptCard = await screen.findByTestId(
      "system-provider-system-chatgpt-subscription",
    );
    const openAiCard = screen.getByTestId("system-provider-system-openai-api");
    expect(within(chatGptCard).queryByRole("spinbutton")).toBeNull();

    const budget = within(openAiCard).getByRole("spinbutton", {
      name: /openai api.*daily budget/i,
    });
    await user.type(budget, "12.34");
    await user.click(within(openAiCard).getByRole("button", { name: "Save budget" }));

    await waitFor(async () => {
      expect(
        (await usageDashboardApi.listProviders()).find(
          (provider) => provider.id === "system-openai-api",
        )?.dailyBudgetUsd,
      ).toBe("12.34");
    });
  });

  it("maps legacy tabs to General and does not query Providers while closed", async () => {
    const providers = vi.spyOn(usageDashboardApi, "listProviders");
    const closed = renderSettings(false, "agents");
    expect(providers).not.toHaveBeenCalled();
    closed.unmount();

    renderSettings(true, "proxy");
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(providers).not.toHaveBeenCalled();
  });
});
