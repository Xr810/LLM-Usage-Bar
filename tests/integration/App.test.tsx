import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "@/App";
import { usageDashboardApi } from "@/lib/api/usageDashboard";

const renderApp = () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
};

describe("App usage-dashboard acceptance with Tauri MSW", () => {
  it("loads backend-defined Agents with unified Provider data and isolated history", async () => {
    const user = userEvent.setup();
    renderApp();

    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getByText("Shared account")).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Metered usage" })).toBeNull();

    const customAgent = await screen.findByRole("tab", {
      name: "Research Agent",
    });
    expect(screen.getAllByRole("tab")).toHaveLength(6);
    await usageDashboardApi.deleteAgentProviderBinding(
      "binding-custom-openrouter",
      0,
    );
    await user.click(customAgent);
    expect(
      await screen.findByText("Research Subscription"),
    ).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();
    expect(screen.queryByText("Shared account")).toBeNull();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.queryByText("Azure API")).toBeNull();
    expect(screen.getAllByText("Recent requests")).toHaveLength(1);
    expect(await screen.findByText("gpt-4.1")).toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Create Custom Agent" }),
    ).toBeNull();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(
      await screen.findByRole("button", { name: "Create Custom Agent" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Proxy setup" }));
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText("legacy-provider")).toBeInTheDocument();
  });

  it("renders one safe notice when the startup boundary is outside Tauri", async () => {
    document.body.innerHTML = '<div id="root"></div>';
    window.localStorage.setItem("llm-usage-bar:language", "en");
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    vi.resetModules();

    await act(async () => {
      await import("@/main");
    });

    expect(
      screen.getAllByText(
        "Current UI preview. Open LLM Usage Bar desktop to read local usage.",
      ),
    ).toHaveLength(1);
    expect(document.body).not.toHaveTextContent("window.__TAURI_INTERNALS__");
    expect(
      fetchSpy.mock.calls.some(([input]) =>
        String(input).match(/list_dashboard_modules|list_usage_providers/),
      ),
    ).toBe(false);
    expect(consoleError).not.toHaveBeenCalled();

    fetchSpy.mockRestore();
    consoleError.mockRestore();
  });
});
