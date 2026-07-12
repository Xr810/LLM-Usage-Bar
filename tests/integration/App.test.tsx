import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
  it("loads five backend-defined modules and keeps Provider data isolated", async () => {
    const user = userEvent.setup();
    renderApp();

    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.queryByText("Azure API")).toBeNull();

    const renamedModule = await screen.findByRole("tab", {
      name: "Renamed research plan",
    });
    expect(screen.getAllByRole("tab")).toHaveLength(5);
    await user.click(renamedModule);
    expect(
      await screen.findByText("Research Subscription"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.queryByText("Azure API")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Metered usage" }));
    expect(await screen.findByText("Azure API")).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();
    expect(screen.queryByText("Research Subscription")).toBeNull();
    expect(screen.getAllByText("Recent requests")).toHaveLength(2);
    expect(await screen.findByText("claude-sonnet-4")).toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Create module" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(
      await screen.findByRole("button", { name: "Create module" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Provider" }));
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Proxy & routing" }));
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
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
