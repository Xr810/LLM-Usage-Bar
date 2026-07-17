import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitTauriEvent } from "../../tests/msw/tauriMocks";
import type { MainWindowDestination } from "@/types/trayUsage";
import { useMainWindowNavigation } from "./useMainWindowNavigation";

const navigationMocks = vi.hoisted(() => ({
  takePending: vi.fn<() => Promise<MainWindowDestination | null>>(),
}));

vi.mock("@/lib/api/trayUsage", () => ({
  takePendingMainWindowDestination: navigationMocks.takePending,
}));

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

  it("treats legacy Agent-targeted usage destinations as Provider dashboard navigation", async () => {
    const openUsage = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "usage",
      agentModuleId: "opencode",
    });

    renderHook(() =>
      useMainWindowNavigation({ openUsage, openProviderSettings: vi.fn() }),
    );

    await waitFor(() => expect(openUsage).toHaveBeenCalledWith());
    expect(openUsage).toHaveBeenCalledOnce();
  });

  it("opens Provider settings for exact and null Provider targets", async () => {
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

  it("serializes overlapping destination drains", async () => {
    const firstDrain = deferred<MainWindowDestination | null>();
    const openUsage = vi.fn();
    const openProviderSettings = vi.fn();
    navigationMocks.takePending
      .mockImplementationOnce(() => firstDrain.promise)
      .mockResolvedValueOnce({ kind: "usage", agentModuleId: "codex" })
      .mockResolvedValueOnce(null);

    renderHook(() =>
      useMainWindowNavigation({ openUsage, openProviderSettings }),
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
      expect(openUsage).toHaveBeenCalledWith();
    });
  });
});
