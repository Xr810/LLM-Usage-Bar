import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitTauriEvent } from "../../tests/msw/tauriMocks";
import type { MainWindowDestination } from "@/types/trayUsage";
import { useMainWindowNavigation } from "./useMainWindowNavigation";

const navigationMocks = vi.hoisted(() => ({
  takePending: vi.fn<() => Promise<MainWindowDestination | null>>(),
  acknowledge: vi.fn<() => Promise<void>>(),
}));

vi.mock("@/lib/api/trayUsage", () => ({
  takePendingMainWindowDestination: navigationMocks.takePending,
  acknowledgeMainWindowReady: navigationMocks.acknowledge,
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
    navigationMocks.acknowledge.mockReset().mockResolvedValue(undefined);
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

  it("opens General settings for a native handoff", async () => {
    const openGeneralSettings = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "generalSettings",
    });

    renderHook(() =>
      useMainWindowNavigation({
        openUsage: vi.fn(),
        openProviderSettings: vi.fn(),
        openGeneralSettings,
      }),
    );

    await waitFor(() => expect(openGeneralSettings).toHaveBeenCalledOnce());
  });

  it("releases the waiting reveal only once the destination is painted", async () => {
    // The window is held back until this lands, so it has to survive the
    // re-render that removing the destination triggers.
    const openProviderSettings = vi.fn();
    navigationMocks.takePending.mockResolvedValueOnce({
      kind: "providerBudget",
      providerId: "system-openai-api",
    });

    renderHook(() =>
      useMainWindowNavigation({ openUsage: vi.fn(), openProviderSettings }),
    );

    await waitFor(() =>
      expect(navigationMocks.acknowledge).toHaveBeenCalledOnce(),
    );
    expect(openProviderSettings).toHaveBeenCalledWith("system-openai-api");
    expect(openProviderSettings.mock.invocationCallOrder[0]).toBeLessThan(
      navigationMocks.acknowledge.mock.invocationCallOrder[0],
    );
  });

  it("does not release the reveal while another destination is still queued", async () => {
    const openProviderSettings = vi.fn();
    navigationMocks.takePending
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({ kind: "providerBudget", providerId: "a" })
      .mockResolvedValueOnce({ kind: "providerBudget", providerId: "b" });

    renderHook(() =>
      useMainWindowNavigation({ openUsage: vi.fn(), openProviderSettings }),
    );
    await waitFor(() =>
      expect(navigationMocks.takePending).toHaveBeenCalledTimes(1),
    );

    act(() => {
      emitTauriEvent("main-window-navigate");
      emitTauriEvent("main-window-navigate");
    });

    await waitFor(() =>
      expect(openProviderSettings).toHaveBeenLastCalledWith("b"),
    );
    await waitFor(() =>
      expect(navigationMocks.acknowledge).toHaveBeenCalledOnce(),
    );
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
