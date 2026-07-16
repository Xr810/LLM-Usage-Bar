import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import {
  getTrayUsageSnapshot,
  hideTrayPopover,
  openMainFromTray,
  quitFromTray,
  setProviderDailyBudget,
  takePendingMainWindowDestination,
} from "@/lib/api/trayUsage";
import {
  trayUsageKeys,
  useRefreshTrayUsage,
  useSetProviderDailyBudget,
  useTrayUsageEventBridge,
  useTrayUsageSnapshot,
} from "@/lib/query/trayUsage";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import {
  setPendingMainWindowDestination,
  trayUsageSnapshotFixture,
} from "../../../tests/msw/handlers";
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

  it("models the cold main-window destination as mutable consume-once state", async () => {
    setPendingMainWindowDestination({
      kind: "providerBudget",
      providerId: "system-openrouter-api",
    });

    await expect(takePendingMainWindowDestination()).resolves.toEqual({
      kind: "providerBudget",
      providerId: "system-openrouter-api",
    });
    await expect(takePendingMainWindowDestination()).resolves.toBeNull();
  });

  it("sends the dedicated daily-budget command without a version or numeric coercion", async () => {
    await setProviderDailyBudget("system-openrouter-api", "025.50");
    await setProviderDailyBudget("system-openrouter-api", null);

    expect(commandCalls("set_provider_daily_budget")).toEqual([
      [
        "set_provider_daily_budget",
        {
          providerId: "system-openrouter-api",
          dailyBudgetUsd: "025.50",
        },
      ],
      [
        "set_provider_daily_budget",
        {
          providerId: "system-openrouter-api",
          dailyBudgetUsd: null,
        },
      ],
    ]);
    expect(
      JSON.stringify(commandCalls("set_provider_daily_budget")),
    ).not.toContain("expectedVersion");
  });
});

describe("tray usage query bridge", () => {
  it("updates the Provider cache and invalidates tray plus dashboard summaries after a budget save", async () => {
    const client = createQueryClient();
    const providers = await usageDashboardApi.listProviders();
    client.setQueryData(usageDashboardKeys.providers(), providers);
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const { result } = renderHook(() => useSetProviderDailyBudget(), {
      wrapper: createQueryWrapper(client),
    });

    await act(async () => {
      await result.current.mutateAsync({
        providerId: "system-openrouter-api",
        dailyBudgetUsd: "17.25",
      });
    });

    expect(
      client
        .getQueryData<
          Awaited<ReturnType<typeof usageDashboardApi.listProviders>>
        >(usageDashboardKeys.providers())
        ?.find((provider) => provider.id === "system-openrouter-api")
        ?.dailyBudgetUsd,
    ).toBe("17.25");
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: trayUsageKeys.all,
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.dashboards(),
    });
  });

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

  it("polls a startup refresh until the backend cache reaches a terminal snapshot", async () => {
    let reads = 0;
    server.use(
      http.post("http://tauri.local/get_tray_usage_snapshot", () => {
        reads += 1;
        return HttpResponse.json({
          ...trayUsageSnapshotFixture,
          refreshInProgress: reads === 1,
        } satisfies TrayUsageSnapshot);
      }),
    );

    const { result } = renderHook(() => useTrayUsageSnapshot(), {
      wrapper: createQueryWrapper(),
    });

    await waitFor(() =>
      expect(result.current.data?.refreshInProgress).toBe(true),
    );
    await waitFor(
      () => expect(result.current.data?.refreshInProgress).toBe(false),
      { timeout: 2_000 },
    );
    expect(reads).toBe(2);
    await new Promise((resolve) => setTimeout(resolve, 600));
    expect(reads).toBe(2);
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
