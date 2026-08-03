import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setPendingMainWindowDestination } from "../tests/msw/handlers";
import { emitTauriEvent } from "../tests/msw/tauriMocks";
import App from "./App";

const windowMocks = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowMocks,
}));

function renderApp() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
}

describe("Provider-only usage dashboard main path", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    Object.values(windowMocks).forEach((mock) => mock.mockReset());
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
  });

  it("renders Provider accounts directly without Agent navigation", async () => {
    renderApp();

    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
    expect(screen.queryByText("Codex", { selector: "[role=tab]" })).toBeNull();
    expect(await screen.findByText("ChatGPT Plus/Pro")).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).toHaveLength(2);
  });

  it("opens General from the global Settings button", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "Provider monitoring" });

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Providers" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Diagnostics" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Agents" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Proxy setup" })).toBeNull();
  });

  it("routes a legacy Provider budget destination to the default shared limit", async () => {
    setPendingMainWindowDestination({
      kind: "providerBudget",
      providerId: "system-openrouter-api",
    });
    renderApp();

    await waitFor(() =>
      expect(document.getElementById("shared-api-budget-input")).toBeInstanceOf(
        HTMLInputElement,
      ),
    );
    expect(
      document.getElementById("provider-budget-input-system-openrouter-api"),
    ).toBeNull();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });

  it("a legacy usage destination only opens the Provider dashboard", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "Provider monitoring" });
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    setPendingMainWindowDestination({
      kind: "usage",
      agentModuleId: "opencode",
    });
    act(() => emitTauriEvent("main-window-navigate"));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      screen.getByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });
});
