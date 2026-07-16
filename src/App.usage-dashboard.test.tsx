import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setPendingMainWindowDestination } from "../tests/msw/handlers";
import { commandCalls, emitTauriEvent } from "../tests/msw/tauriMocks";
import App from "./App";
import { setSettings } from "../tests/msw/state";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { proxyApi } from "@/lib/api/proxy";

const windowMocks = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}));
const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

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
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
  });

  afterEach(() => {
    if (originalScrollIntoView) {
      HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
    } else {
      delete (
        HTMLElement.prototype as Partial<HTMLElement> & {
          scrollIntoView?: HTMLElement["scrollIntoView"];
        }
      ).scrollIntoView;
    }
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

  it("returns to the selected Agent when the Settings close control is clicked", async () => {
    const user = userEvent.setup();
    renderApp();

    const claude = await screen.findByRole("tab", { name: "Claude Code" });
    await user.click(claude);
    expect(claude).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("button", { name: "Settings" }));

    expect(
      await screen.findByRole("heading", { name: "Settings" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Settings" })).toBeNull(),
    );
    expect(claude).toHaveAttribute("aria-selected", "true");
    expect(localStorage.getItem("llm-usage-bar:last-agent-module-id")).toBe(
      "claude-code",
    );
    expect(screen.getByText("Claude Team")).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
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

  it("drains a cold Provider destination into Providers and focuses its budget", async () => {
    setPendingMainWindowDestination({
      kind: "providerBudget",
      providerId: "system-openrouter-api",
    });
    renderApp();

    await waitFor(() =>
      expect(
        document.getElementById("provider-budget-input-system-openrouter-api"),
      ).toBeInstanceOf(HTMLInputElement),
    );
    const input = document.getElementById(
      "provider-budget-input-system-openrouter-api",
    );
    if (!(input instanceof HTMLInputElement)) {
      throw new Error("targeted Provider budget input was not rendered");
    }
    await waitFor(() => expect(input).toHaveFocus());
    expect(screen.getByRole("tab", { name: "Providers" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("opens untargeted Provider settings without focusing a budget field", async () => {
    setPendingMainWindowDestination({
      kind: "providerBudget",
      providerId: null,
    });
    renderApp();

    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Providers" })).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
    const budgetInputs = await screen.findAllByRole("spinbutton");
    expect(budgetInputs.some((input) => input === document.activeElement)).toBe(
      false,
    );
  });

  it("applies a live exact-Agent destination once and closes Settings", async () => {
    const user = userEvent.setup();
    renderApp();

    await screen.findByRole("tab", { name: "Codex" });
    await waitFor(() =>
      expect(commandCalls("take_pending_main_window_destination")).toHaveLength(
        1,
      ),
    );
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    setPendingMainWindowDestination({
      kind: "usage",
      agentModuleId: "opencode",
    });
    act(() => emitTauriEvent("main-window-navigate"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.getByRole("tab", { name: "OpenCode" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
    expect(localStorage.getItem("llm-usage-bar:last-agent-module-id")).toBe(
      "opencode",
    );
  });

  it("manual Settings always returns to Agents and clears an old Provider target", async () => {
    const user = userEvent.setup();
    renderApp();

    await screen.findByRole("tab", { name: "Codex" });
    await waitFor(() =>
      expect(commandCalls("take_pending_main_window_destination")).toHaveLength(
        1,
      ),
    );
    setPendingMainWindowDestination({
      kind: "providerBudget",
      providerId: "system-openrouter-api",
    });
    act(() => emitTauriEvent("main-window-navigate"));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Providers" })).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
  });
});
