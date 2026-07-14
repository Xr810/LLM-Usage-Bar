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

    expect(await screen.findByDisplayValue("Research Agent")).toBeInTheDocument();
    expect(screen.getAllByText("Official Subscription")).not.toHaveLength(0);
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
    expect(await screen.findByText("Official Subscription")).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Proxy setup" }));
    expect(await screen.findByText("Codex setup")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:15800")).toBeInTheDocument();
    expect(screen.queryByText("https://azure.example.com")).toBeNull();
    expect(screen.getByRole("button", { name: /^(Start|Stop) proxy$/ })).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText("legacy-provider")).toBeInTheDocument();
    expect(screen.getByText("custom-archived")).toBeInTheDocument();
    expect(screen.getByText("cross_agent")).toBeInTheDocument();
    expect(screen.queryByText("requestId", { exact: false })).toBeNull();
  });

  it("keeps Agent and protected-key mutations observable without retaining the key", async () => {
    const user = userEvent.setup();
    const view = renderSettings();
    await screen.findByDisplayValue("Research Agent");

    await user.type(screen.getByLabelText("Custom Agent name"), "MSW Custom");
    await user.click(screen.getByRole("button", { name: "Create Custom Agent" }));
    expect(await screen.findByDisplayValue("MSW Custom")).toBeInTheDocument();

    await user.selectOptions(
      screen.getByRole("combobox", { name: "Add Provider for Hermes" }),
      "openrouter-api",
    );
    await user.click(screen.getByRole("button", { name: "Add binding for Hermes" }));

    const binding = await screen.findByTestId("agent-binding-binding-msw-1");
    expect(within(binding).getByText("Missing")).toBeInTheDocument();
    await user.click(within(binding).getByRole("button", { name: "Set API key" }));

    const dialog = screen.getByRole("dialog");
    const input = within(dialog).getByLabelText("API key");
    const bindingKey = "transient-msw-key";
    await user.type(input, bindingKey);
    await user.click(within(dialog).getByRole("button", { name: "Set API key" }));

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

  it.each(["general", "advanced", "mcp", "about", "unknown", "modules"])(
    "maps historical or unknown tab %s to Agents",
    async (defaultTab) => {
      renderSettings(true, defaultTab);
      expect(await screen.findByDisplayValue("Research Agent")).toBeInTheDocument();
      expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    },
  );

  it("honors Provider, Proxy, and Diagnostics defaults", async () => {
    const providerView = renderSettings(true, "providers");
    expect(await screen.findByText("Official Subscription")).toBeInTheDocument();
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
