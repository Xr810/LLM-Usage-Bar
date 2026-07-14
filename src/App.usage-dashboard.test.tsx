import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { setSettings } from "../tests/msw/state";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { proxyApi } from "@/lib/api/proxy";

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

describe("Agent usage dashboard main path", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    Object.values(windowMocks).forEach((mock) => mock.mockReset());
  });

  it("renders the five fixed Agents plus Custom without a global API tab", async () => {
    renderApp();

    expect(
      (await screen.findAllByRole("tab")).map((tab) => tab.textContent),
    ).toEqual([
      "Codex",
      "Claude Code",
      "OpenCode",
      "OpenClaw",
      "Hermes",
      "Research Agent",
    ]);
    expect(screen.queryByRole("tab", { name: /API/i })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Metered usage" })).toBeNull();
    expect(screen.getByRole("tablist", { name: "Agents" })).toBeInTheDocument();
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
  });

  it("treats Agent tab changes as pure selection with zero mutations", async () => {
    const user = userEvent.setup();
    const mutations = [
      vi.spyOn(usageDashboardApi, "saveAgentModule"),
      vi.spyOn(usageDashboardApi, "setAgentModuleVisibility"),
      vi.spyOn(usageDashboardApi, "deleteAgentModule"),
      vi.spyOn(usageDashboardApi, "saveProvider"),
      vi.spyOn(usageDashboardApi, "setProviderEnabled"),
      vi.spyOn(usageDashboardApi, "saveAgentProviderBinding"),
      vi.spyOn(usageDashboardApi, "deleteAgentProviderBinding"),
      vi.spyOn(usageDashboardApi, "setAgentProviderBindingApiKey"),
      vi.spyOn(usageDashboardApi, "replaceAgentProviderBindingApiKey"),
      vi.spyOn(usageDashboardApi, "clearAgentProviderBindingApiKey"),
      vi.spyOn(proxyApi, "startProxyServer"),
      vi.spyOn(proxyApi, "stopProxyWithRestore"),
    ];
    renderApp();

    const codex = await screen.findByRole("tab", { name: "Codex" });
    const claude = screen.getByRole("tab", { name: "Claude Code" });
    expect(codex).toHaveAttribute("aria-selected", "true");

    await user.click(claude);
    expect(claude).toHaveAttribute("aria-selected", "true");
    for (const mutation of mutations) {
      expect(mutation).not.toHaveBeenCalled();
    }
  });

  it("selects a Custom Agent by stable ID without adding frontend routes", async () => {
    const user = userEvent.setup();
    renderApp();

    const custom = await screen.findByRole("tab", { name: "Research Agent" });
    await user.click(custom);

    expect(custom).toHaveAttribute("aria-selected", "true");
    expect(localStorage.getItem("llm-usage-bar:last-agent-module-id")).toBe(
      "custom-research",
    );
  });

  it("keeps a draggable title area and gates native window controls by settings", async () => {
    setSettings({ useAppWindowControls: true, language: "en" });
    const { container } = renderApp();

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Minimize window" }),
      ).toBeInTheDocument(),
    );
    const dragRegion = container.querySelector("[data-tauri-drag-region]");
    expect(dragRegion).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Minimize window" }));
    fireEvent.click(screen.getByRole("button", { name: "Maximize window" }));
    fireEvent.click(screen.getByRole("button", { name: "Close window" }));
    expect(windowMocks.minimize).toHaveBeenCalledOnce();
    expect(windowMocks.toggleMaximize).toHaveBeenCalledOnce();
    expect(windowMocks.close).toHaveBeenCalledOnce();
  });
});
