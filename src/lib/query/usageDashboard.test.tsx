import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import {
  usageDashboardKeys,
  useDashboardModules,
  useDeleteDashboardModule,
  useRefreshProviderQuota,
  useReorderDashboardModules,
  useSaveDashboardModule,
  useSaveUsageProvider,
  useSetDashboardModuleVisibility,
  useSetRouteBinding,
} from "@/lib/query/usageDashboard";
import type {
  DashboardModuleInput,
  UsageProviderInput,
} from "@/types/usageDashboard";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const input: UsageProviderInput = {
  id: "provider-a",
  name: "Provider A",
  billingKind: "metered",
  productGroupId: "claude",
  tokenSources: ["proxy"],
  sessionSourceBindings: [],
  quotaSource: null,
  quotaIntervalSeconds: null,
  routeAppType: "claude",
  routeConfig: { baseUrl: "https://example.com", apiKey: "secret" },
  quotaConfig: null,
  enabled: true,
};

const moduleInput: DashboardModuleInput = {
  id: null,
  name: "Gemini",
  kind: "subscription",
  sortOrder: 4,
  visible: true,
};

describe("usageDashboardApi wire contract", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockResolvedValue(undefined);
  });

  it("sends exact camelCase payloads for all nine commands", async () => {
    await usageDashboardApi.listProviders();
    await usageDashboardApi.saveProvider(input);
    await usageDashboardApi.setProviderEnabled("provider-a", false);
    await usageDashboardApi.getRouteBindings();
    await usageDashboardApi.setRouteBinding("claude", "provider-a");
    await usageDashboardApi.getDashboard(10, 20, "claude");
    await usageDashboardApi.getEvents("provider-a", 10, 20, 2, 50);
    await usageDashboardApi.refreshQuota("provider-a");
    await usageDashboardApi.syncSessionUsage("provider-a");

    expect(invokeMock.mock.calls).toEqual([
      ["list_usage_providers"],
      ["save_usage_provider", { input }],
      [
        "set_usage_provider_enabled",
        { providerId: "provider-a", enabled: false },
      ],
      ["get_route_bindings"],
      ["set_route_binding", { protocol: "claude", providerId: "provider-a" }],
      [
        "get_usage_dashboard",
        { startAt: 10, endAt: 20, productGroupId: "claude" },
      ],
      [
        "get_usage_events",
        {
          providerId: "provider-a",
          startAt: 10,
          endAt: 20,
          page: 2,
          pageSize: 50,
        },
      ],
      ["refresh_provider_quota", { providerId: "provider-a" }],
      ["sync_provider_session_usage", { providerId: "provider-a" }],
    ]);
  });

  it("builds deterministic keys from every range filter and page value", () => {
    expect(usageDashboardKeys.dashboard(10, 20, "claude")).toEqual([
      "usage-dashboard",
      "dashboard",
      10,
      20,
      "claude",
    ]);
    expect(usageDashboardKeys.events("provider-a", 10, 20, 2, 50)).toEqual([
      "usage-dashboard",
      "events",
      "provider-a",
      10,
      20,
      2,
      50,
    ]);
  });

  it("sends exact camelCase payloads for dashboard module commands", async () => {
    await usageDashboardApi.listDashboardModules();
    await usageDashboardApi.saveDashboardModule(moduleInput);
    await usageDashboardApi.reorderDashboardModules(["codex", "api"]);
    await usageDashboardApi.setDashboardModuleVisibility("codex", false);
    await usageDashboardApi.deleteDashboardModule("gemini");

    expect(invokeMock.mock.calls).toEqual([
      ["list_dashboard_modules"],
      ["save_dashboard_module", { input: moduleInput }],
      ["reorder_dashboard_modules", { moduleIds: ["codex", "api"] }],
      [
        "set_dashboard_module_visibility",
        { moduleId: "codex", visible: false },
      ],
      ["delete_dashboard_module", { moduleId: "gemini" }],
    ]);
  });

  it("uses a stable modules query key", () => {
    expect(usageDashboardKeys.modules()).toEqual([
      "usage-dashboard",
      "modules",
    ]);
  });
});

function wrapper(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("usage dashboard mutation invalidation", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({}));

  it("invalidates providers, bindings and dashboard after provider and route edits", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const save = renderHook(() => useSaveUsageProvider(), {
      wrapper: wrapper(client),
    });
    const route = renderHook(() => useSetRouteBinding(), {
      wrapper: wrapper(client),
    });

    await act(async () => save.result.current.mutateAsync(input));
    await act(async () =>
      route.result.current.mutateAsync({
        protocol: "claude",
        providerId: "provider-a",
      }),
    );

    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.providers(),
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.bindings(),
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.dashboards(),
    });
  });

  it("invalidates the provider and dashboard after quota refresh", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const refresh = renderHook(() => useRefreshProviderQuota(), {
      wrapper: wrapper(client),
    });

    await act(async () => refresh.result.current.mutateAsync("provider-a"));

    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.provider("provider-a"),
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.dashboards(),
    });
  });

  it("loads dashboard modules through the modules query key", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const modules = renderHook(() => useDashboardModules(), {
      wrapper: wrapper(client),
    });

    await waitFor(() => expect(modules.result.current.isSuccess).toBe(true));
    expect(invokeMock).toHaveBeenCalledWith("list_dashboard_modules");
    expect(client.getQueryState(["usage-dashboard", "modules"])).toBeDefined();
  });

  it("invalidates modules, providers and dashboards after module mutations", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const save = renderHook(() => useSaveDashboardModule(), {
      wrapper: wrapper(client),
    });
    const reorder = renderHook(() => useReorderDashboardModules(), {
      wrapper: wrapper(client),
    });
    const visibility = renderHook(() => useSetDashboardModuleVisibility(), {
      wrapper: wrapper(client),
    });
    const remove = renderHook(() => useDeleteDashboardModule(), {
      wrapper: wrapper(client),
    });

    await act(async () => save.result.current.mutateAsync(moduleInput));
    await act(async () => reorder.result.current.mutateAsync(["codex", "api"]));
    await act(async () =>
      visibility.result.current.mutateAsync({
        moduleId: "codex",
        visible: false,
      }),
    );
    await act(async () => remove.result.current.mutateAsync("gemini"));

    for (const queryKey of [
      ["usage-dashboard", "modules"],
      usageDashboardKeys.providers(),
      usageDashboardKeys.dashboards(),
    ]) {
      expect(invalidate).toHaveBeenCalledWith({ queryKey });
    }
  });
});
