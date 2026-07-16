import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitTauriEvent } from "../../tests/msw/tauriMocks";
import type { MainWindowDestination } from "@/types/trayUsage";
import type { AgentModuleView } from "@/types/usageDashboard";
import { useMainWindowNavigation } from "./useMainWindowNavigation";

const navigationMocks = vi.hoisted(() => ({
  takePending: vi.fn<() => Promise<MainWindowDestination | null>>(),
}));

vi.mock("@/lib/api/trayUsage", () => ({
  takePendingMainWindowDestination: navigationMocks.takePending,
}));

function agent(
  id: string,
  overrides: Partial<AgentModuleView> = {},
): AgentModuleView {
  return {
    id,
    name: id,
    sortOrder: 1,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 0,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe("useMainWindowNavigation", () => {
  beforeEach(() => {
    navigationMocks.takePending.mockReset().mockResolvedValue(null);
  });

  it("consumes a cold destination and opens the exact eligible Agent once", async () => {
    const openUsage = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "usage",
      agentModuleId: "opencode",
    });

    const { rerender } = renderHook(
      ({ agents }) =>
        useMainWindowNavigation({
          agents,
          openUsage,
          openProviderSettings: vi.fn(),
        }),
      { initialProps: { agents: [agent("codex"), agent("opencode")] } },
    );

    await waitFor(() => expect(openUsage).toHaveBeenCalledWith("opencode"));
    rerender({ agents: [agent("codex"), agent("opencode")] });
    expect(openUsage).toHaveBeenCalledOnce();
  });

  it("waits for the requested Agent to be visible and active without falling back", async () => {
    const openUsage = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "usage",
      agentModuleId: "opencode",
    });

    const { rerender } = renderHook(
      ({ agents }) =>
        useMainWindowNavigation({
          agents,
          openUsage,
          openProviderSettings: vi.fn(),
        }),
      { initialProps: { agents: [agent("codex")] } },
    );

    await waitFor(() => expect(navigationMocks.takePending).toHaveBeenCalled());
    expect(openUsage).not.toHaveBeenCalled();

    rerender({
      agents: [agent("codex"), agent("opencode", { visible: false })],
    });
    expect(openUsage).not.toHaveBeenCalled();

    rerender({
      agents: [agent("codex"), agent("opencode", { archivedAt: 1_000 })],
    });
    expect(openUsage).not.toHaveBeenCalled();

    rerender({ agents: [agent("codex"), agent("opencode")] });
    await waitFor(() => expect(openUsage).toHaveBeenCalledWith("opencode"));
    expect(openUsage).toHaveBeenCalledOnce();
  });

  it("executes a later actionable target while preserving a blocked Agent target", async () => {
    const openUsage = vi.fn();
    const openProviderSettings = vi.fn();
    navigationMocks.takePending
      .mockResolvedValueOnce({ kind: "usage", agentModuleId: "opencode" })
      .mockResolvedValueOnce({
        kind: "providerBudget",
        providerId: "system-openrouter-api",
      });

    const { rerender } = renderHook(
      ({ agents }) =>
        useMainWindowNavigation({
          agents,
          openUsage,
          openProviderSettings,
        }),
      { initialProps: { agents: [agent("codex")] } },
    );
    await waitFor(() =>
      expect(navigationMocks.takePending).toHaveBeenCalledTimes(1),
    );
    expect(openUsage).not.toHaveBeenCalled();

    act(() => emitTauriEvent("main-window-navigate"));
    await waitFor(() =>
      expect(openProviderSettings).toHaveBeenCalledWith(
        "system-openrouter-api",
      ),
    );
    expect(openProviderSettings).toHaveBeenCalledOnce();
    expect(openUsage).not.toHaveBeenCalled();

    rerender({
      agents: [agent("codex"), agent("opencode", { visible: false })],
    });
    expect(openUsage).not.toHaveBeenCalled();

    rerender({ agents: [agent("codex"), agent("opencode")] });
    await waitFor(() => expect(openUsage).toHaveBeenCalledWith("opencode"));
    expect(openUsage).toHaveBeenCalledOnce();
    expect(openProviderSettings).toHaveBeenCalledOnce();
  });

  it("opens untargeted usage without selecting a fallback Agent", async () => {
    const openUsage = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "usage",
      agentModuleId: null,
    });

    renderHook(() =>
      useMainWindowNavigation({
        agents: [agent("codex")],
        openUsage,
        openProviderSettings: vi.fn(),
      }),
    );

    await waitFor(() => expect(openUsage).toHaveBeenCalledWith(null));
    expect(openUsage).toHaveBeenCalledOnce();
  });

  it("consumes live Provider destinations including a null focus target", async () => {
    const openProviderSettings = vi.fn();
    navigationMocks.takePending
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        kind: "providerBudget",
        providerId: "system-openrouter-api",
      })
      .mockResolvedValueOnce({ kind: "providerBudget", providerId: null });

    renderHook(() =>
      useMainWindowNavigation({
        agents: [agent("codex")],
        openUsage: vi.fn(),
        openProviderSettings,
      }),
    );
    await waitFor(() =>
      expect(navigationMocks.takePending).toHaveBeenCalledTimes(1),
    );

    act(() => emitTauriEvent("main-window-navigate"));
    await waitFor(() =>
      expect(openProviderSettings).toHaveBeenCalledWith(
        "system-openrouter-api",
      ),
    );

    act(() => emitTauriEvent("main-window-navigate"));
    await waitFor(() =>
      expect(openProviderSettings).toHaveBeenLastCalledWith(null),
    );
    expect(openProviderSettings).toHaveBeenCalledTimes(2);
  });

  it("serializes overlapping drains and executes every consumed target once", async () => {
    const firstDrain = deferred<MainWindowDestination | null>();
    const openUsage = vi.fn();
    const openProviderSettings = vi.fn();
    navigationMocks.takePending
      .mockImplementationOnce(() => firstDrain.promise)
      .mockResolvedValueOnce({
        kind: "usage",
        agentModuleId: "opencode",
      })
      .mockResolvedValueOnce(null);

    renderHook(() =>
      useMainWindowNavigation({
        agents: [agent("codex"), agent("opencode")],
        openUsage,
        openProviderSettings,
      }),
    );
    await waitFor(() =>
      expect(navigationMocks.takePending).toHaveBeenCalledTimes(1),
    );

    act(() => {
      emitTauriEvent("main-window-navigate");
      emitTauriEvent("main-window-navigate");
    });
    expect(navigationMocks.takePending).toHaveBeenCalledTimes(1);

    act(() => {
      firstDrain.resolve({
        kind: "providerBudget",
        providerId: "system-openai-api",
      });
    });

    await waitFor(() => {
      expect(navigationMocks.takePending).toHaveBeenCalledTimes(3);
      expect(openProviderSettings).toHaveBeenCalledWith("system-openai-api");
      expect(openUsage).toHaveBeenCalledWith("opencode");
    });
    expect(openProviderSettings).toHaveBeenCalledOnce();
    expect(openUsage).toHaveBeenCalledOnce();
  });
});
