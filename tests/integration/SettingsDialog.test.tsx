import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { proxyApi } from "@/lib/api/proxy";

const renderSettings = (open = true, defaultTab?: string) => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <SettingsPage
        open={open}
        onOpenChange={() => {}}
        defaultTab={defaultTab}
      />
    </QueryClientProvider>,
  );
  return { ...view, client };
};

describe("SettingsPage integration", () => {
  it("loads four isolated Agent-centric sections through Tauri MSW", async () => {
    const user = userEvent.setup();
    renderSettings();

    await user.click(await screen.findByTestId("agent-sort-custom-research"));
    expect(
      await screen.findByDisplayValue("Research Agent"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Official Subscription")).not.toHaveLength(0);
    await user.click(screen.getByTestId("agent-sort-opencode"));
    expect(
      within(screen.getByTestId("agent-settings-opencode")).getByText(
        "2 Providers",
      ),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByTestId("agent-binding-binding-opencode-subscription"),
      ).getByText("Effective: Disabled"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).not.toHaveLength(0);

    await user.click(screen.getByRole("tab", { name: "Proxy setup" }));
    expect(await screen.findByText("Codex setup")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:15800")).toBeInTheDocument();
    expect(screen.queryByText("https://azure.example.com")).toBeNull();
    expect(
      screen.getByRole("button", { name: /^(Start|Stop) proxy$/ }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText("legacy-provider")).toBeInTheDocument();
    expect(screen.getByText("custom-archived")).toBeInTheDocument();
    expect(screen.getByText("cross_agent")).toBeInTheDocument();
    expect(screen.queryByText("requestId", { exact: false })).toBeNull();
  });

  it("keeps Agent and protected-key mutations observable without retaining the key", async () => {
    const user = userEvent.setup();
    const view = renderSettings();
    await screen.findByTestId("agent-sort-custom-research");

    await user.type(screen.getByLabelText("Custom Agent name"), "MSW Custom");
    await user.click(
      screen.getByRole("button", { name: "Create Custom Agent" }),
    );
    expect(await screen.findByText("MSW Custom")).toBeInTheDocument();

    await user.click(screen.getByTestId("agent-sort-hermes"));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Add Provider for Hermes" }),
      "openrouter-api",
    );
    await user.click(
      screen.getByRole("button", { name: "Add binding for Hermes" }),
    );

    const binding = await screen.findByTestId("agent-binding-binding-msw-1");
    expect(within(binding).getByText("Missing")).toBeInTheDocument();
    await user.click(
      within(binding).getByRole("button", { name: "Set API key" }),
    );

    const dialog = screen.getByRole("dialog", { name: "Set API key" });
    const input = within(dialog).getByLabelText("API key");
    const bindingKey = "transient-msw-key";
    await user.type(input, bindingKey);
    await user.click(
      within(dialog).getByRole("button", { name: "Set API key" }),
    );

    await waitFor(() =>
      expect(
        within(screen.getByTestId("agent-binding-binding-msw-1")).getByText(
          "Configured",
        ),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByDisplayValue(bindingKey)).toBeNull();

    const frontendSnapshot = JSON.stringify({
      dom: view.baseElement.innerHTML,
      formValues: Array.from(
        view.baseElement.querySelectorAll<
          HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
        >("input, textarea, select"),
        (element) => element.value,
      ),
      queries: view.client
        .getQueryCache()
        .getAll()
        .map((query) => query.state.data),
      mutations: view.client
        .getMutationCache()
        .getAll()
        .map((mutation) => ({
          data: mutation.state.data,
          error: mutation.state.error,
          variables: mutation.state.variables,
          context: mutation.state.context,
        })),
    });
    expect(frontendSnapshot).not.toContain(bindingKey);
  });

  it("renders immutable system cards first and never retains an upstream Provider key", async () => {
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
    }
    expect(
      within(
        screen.getByTestId("provider-config-subscription-official"),
      ).getByRole("button", { name: "Edit Official Subscription" }),
    ).toBeInTheDocument();

    const openAiCard = screen.getByTestId("system-provider-system-openai-api");
    const openRouterCard = screen.getByTestId(
      "system-provider-system-openrouter-api",
    );
    for (const agent of ["OpenCode", "OpenClaw", "Hermes"]) {
      expect(
        within(openRouterCard).getByLabelText(`Use ${agent}`),
      ).toBeChecked();
    }
    for (const agent of ["Claude Code", "Codex"]) {
      expect(
        within(openRouterCard).getByLabelText(`Use ${agent}`),
      ).not.toBeChecked();
    }
    expect(
      within(openAiCard).getByText("https://api.openai.com/v1"),
    ).toBeInTheDocument();
    await user.click(
      within(openAiCard).getByRole("button", { name: "Set API key" }),
    );
    const dialog = screen.getByRole("dialog");
    const upstreamKey = "transient-upstream-provider-sentinel";
    await user.type(within(dialog).getByLabelText("API key"), upstreamKey);
    await user.click(
      within(dialog).getByRole("button", { name: "Save API key" }),
    );
    await waitFor(() =>
      expect(
        within(openAiCard).getByText("Upstream API key configured"),
      ).toBeInTheDocument(),
    );

    const frontendSnapshot = JSON.stringify({
      dom: view.baseElement.innerHTML,
      formValues: Array.from(
        view.baseElement.querySelectorAll<
          HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
        >("input, textarea, select"),
        (element) => element.value,
      ),
      queries: view.client
        .getQueryCache()
        .getAll()
        .map((query) => query.state.data),
      mutations: view.client
        .getMutationCache()
        .getAll()
        .map((mutation) => ({
          data: mutation.state.data,
          error: mutation.state.error,
          variables: mutation.state.variables,
          context: mutation.state.context,
        })),
    });
    expect(frontendSnapshot).not.toContain(upstreamKey);
  });

  it("edits daily budgets only on metered Providers through the dedicated command", async () => {
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
    await user.click(
      within(openAiCard).getByRole("button", { name: "Save budget" }),
    );

    await waitFor(async () => {
      expect(
        (await usageDashboardApi.listProviders()).find(
          (provider) => provider.id === "system-openai-api",
        )?.dailyBudgetUsd,
      ).toBe("12.34");
    });

    await user.click(
      within(openAiCard).getByRole("button", { name: "Clear budget" }),
    );
    await waitFor(async () => {
      expect(
        (await usageDashboardApi.listProviders()).find(
          (provider) => provider.id === "system-openai-api",
        )?.dailyBudgetUsd,
      ).toBeNull();
    });
  });

  it.each(["general", "advanced", "mcp", "about", "unknown", "modules"])(
    "maps historical or unknown tab %s to Agents",
    async (defaultTab) => {
      renderSettings(true, defaultTab);
      expect(
        await screen.findByTestId("agent-sort-custom-research"),
      ).toBeInTheDocument();
      expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    },
  );

  it("honors Provider, Proxy, and Diagnostics defaults", async () => {
    const providerView = renderSettings(true, "providers");
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    providerView.unmount();

    const proxyView = renderSettings(true, "proxy");
    expect(await screen.findByText("Codex setup")).toBeInTheDocument();
    proxyView.unmount();

    renderSettings(true, "diagnostics");
    expect(await screen.findByText("legacy-provider")).toBeInTheDocument();
  });

  it("does not start configuration queries while closed", () => {
    const agents = vi.spyOn(usageDashboardApi, "listAgentModules");
    const providers = vi.spyOn(usageDashboardApi, "listProviders");
    const bindings = vi.spyOn(usageDashboardApi, "listAgentProviderBindings");
    const proxy = vi.spyOn(proxyApi, "isProxyRunning");

    renderSettings(false);

    expect(agents).not.toHaveBeenCalled();
    expect(providers).not.toHaveBeenCalled();
    expect(bindings).not.toHaveBeenCalled();
    expect(proxy).not.toHaveBeenCalled();
  });
});
