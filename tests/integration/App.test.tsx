import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "@/App";

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
  it("loads Provider accounts directly with unified usage and no Agent controls", async () => {
    renderApp();

    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getAllByText("25% used")).not.toHaveLength(0);
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).not.toHaveLength(0);
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
    expect(screen.queryByText("Shared account")).toBeNull();
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
