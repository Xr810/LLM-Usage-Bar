import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import {
  getTrayUsageSnapshot,
  hideTrayPopover,
  openMainFromTray,
  quitFromTray,
  takePendingMainWindowDestination,
} from "@/lib/api/trayUsage";
import {
  trayUsageKeys,
  useRefreshTrayUsage,
  useTrayUsageEventBridge,
  useTrayUsageSnapshot,
} from "@/lib/query/trayUsage";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { trayUsageSnapshotFixture } from "../../../tests/msw/handlers";
import { server } from "../../../tests/msw/server";
import {
  commandCalls,
  emitTauriEvent,
  tauriInvoke,
} from "../../../tests/msw/tauriMocks";

function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
}

function createQueryWrapper(client = createQueryClient()) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("tray usage wire contract", () => {
  it("mirrors decimal fields as strings and excludes protected quota configuration", () => {
    const subscription = trayUsageSnapshotFixture.agents[0].providers[0];
    const metered = trayUsageSnapshotFixture.agents[0].providers[1];

    expect(subscription.subscription?.windows[0].remainingPercent).toBe("88");
    expect(metered.metered?.todayCostUsd).toBe("2.5");
    expect(metered.metered?.dailyBudgetUsd).toBe("10");
    expect(JSON.stringify(trayUsageSnapshotFixture)).not.toContain(
      "quotaConfig",
    );
  });

  it("invokes the exact tray command names and destination argument", async () => {
    await getTrayUsageSnapshot();
    await hideTrayPopover();
    await openMainFromTray({ kind: "usage", agentModuleId: "codex" });
    await takePendingMainWindowDestination();
    await quitFromTray();

    expect(tauriInvoke.mock.calls).toEqual([
      ["get_tray_usage_snapshot"],
      ["hide_tray_popover"],
      [
        "open_main_from_tray",
        { destination: { kind: "usage", agentModuleId: "codex" } },
      ],
      ["take_pending_main_window_destination"],
      ["quit_from_tray"],
    ]);
  });
});

describe("tray usage query bridge", () => {
  it("publishes an event payload directly into the snapshot cache", async () => {
    const wrapper = createQueryWrapper();
    const { result } = renderHook(
      () => {
        useTrayUsageEventBridge();
        return useTrayUsageSnapshot();
      },
      { wrapper },
    );

    await waitFor(() => expect(result.current.data?.status).toBe("green"));

    act(() => {
      emitTauriEvent("tray-usage-updated", {
        ...trayUsageSnapshotFixture,
        status: "red",
        generatedAt: 2_000,
      } satisfies TrayUsageSnapshot);
    });

    await waitFor(() => expect(result.current.data?.status).toBe("red"));
    expect(tauriInvoke).toHaveBeenCalledTimes(1);
    expect(commandCalls("get_tray_usage_snapshot")).toHaveLength(1);
  });

  it("deduplicates consecutive refresh mutations into one IPC", async () => {
    const client = createQueryClient();
    const { result } = renderHook(() => useRefreshTrayUsage(), {
      wrapper: createQueryWrapper(client),
    });

    act(() => {
      result.current.mutate();
      result.current.mutate();
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(commandCalls("refresh_tray_usage")).toHaveLength(1);
    expect(client.getQueryData(trayUsageKeys.snapshot())).toEqual(
      trayUsageSnapshotFixture,
    );
  });

  it("skips refresh IPC while the cached snapshot is already refreshing", async () => {
    const client = createQueryClient();
    const refreshingSnapshot: TrayUsageSnapshot = {
      ...trayUsageSnapshotFixture,
      refreshInProgress: true,
    };
    client.setQueryData(trayUsageKeys.snapshot(), refreshingSnapshot);
    const { result } = renderHook(() => useRefreshTrayUsage(), {
      wrapper: createQueryWrapper(client),
    });

    act(() => result.current.mutate());

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(commandCalls("refresh_tray_usage")).toHaveLength(0);
    expect(client.getQueryData(trayUsageKeys.snapshot())).toEqual(
      refreshingSnapshot,
    );
  });

  it("retains the cached snapshot when refresh fails", async () => {
    server.use(
      http.post("http://tauri.local/refresh_tray_usage", () =>
        HttpResponse.json("refresh_failed", { status: 500 }),
      ),
    );
    const client = createQueryClient();
    client.setQueryData(trayUsageKeys.snapshot(), trayUsageSnapshotFixture);
    const { result } = renderHook(() => useRefreshTrayUsage(), {
      wrapper: createQueryWrapper(client),
    });

    act(() => result.current.mutate());

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(client.getQueryData(trayUsageKeys.snapshot())).toEqual(
      trayUsageSnapshotFixture,
    );
  });
});
