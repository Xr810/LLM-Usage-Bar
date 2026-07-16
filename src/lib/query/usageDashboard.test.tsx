import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { claudeCliAuthApi } from "@/lib/api/claudeCliAuth";
import {
  usageDashboardKeys,
  useAgentProviderBindingCredentialActions,
  useDeleteAgentModule,
  useDeleteAgentProviderBinding,
  useRefreshProviderQuota,
  useReorderAgentModules,
  useSaveAgentModule,
  useSaveAgentProviderBinding,
  useSaveUsageProvider,
  useSetAgentModuleVisibility,
  useSetUsageProviderEnabled,
  useSyncProviderSessionUsage,
  useUsageDashboard,
  useUsageEvents,
} from "@/lib/query/usageDashboard";
import type {
  AgentModuleInput,
  AgentProviderBindingInput,
  UsageProviderInput,
} from "@/types/usageDashboard";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const providerInput: UsageProviderInput = {
  id: "provider-a",
  name: "Provider A",
  billingKind: "metered",
  productGroupId: "claude",
  tokenSources: ["proxy"],
  sessionSourceBindings: [],
  quotaSource: null,
  quotaIntervalSeconds: null,
  routeAppType: "claude",
  routeConfig: { baseUrl: "https://example.com" },
  enabled: true,
};

const agentInput: AgentModuleInput = {
  id: null,
  name: "Custom research agent",
  sortOrder: 5,
  visible: true,
};

const bindingInput: AgentProviderBindingInput = {
  id: "binding-a",
  agentModuleId: "codex",
  providerId: "provider-a",
  enabled: false,
};

describe("usageDashboardApi wire contract", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockResolvedValue(undefined);
  });

  it("includes the selected Agent ID in dashboard and event requests", async () => {
    await usageDashboardApi.getDashboard("codex", 10, 20);
    await usageDashboardApi.getEvents("codex", "provider-a", 10, 20, 2, 50);

    expect(invokeMock.mock.calls).toEqual([
      [
        "get_usage_dashboard",
        { agentModuleId: "codex", startAt: 10, endAt: 20 },
      ],
      [
        "get_usage_events",
        {
          agentModuleId: "codex",
          providerId: "provider-a",
          startAt: 10,
          endAt: 20,
          page: 2,
          pageSize: 50,
        },
      ],
    ]);
  });

  it("uses Agent names for the five compatibility module commands", async () => {
    await usageDashboardApi.listAgentModules();
    await usageDashboardApi.saveAgentModule(agentInput);
    await usageDashboardApi.reorderAgentModules(["codex", "claude-code"]);
    await usageDashboardApi.setAgentModuleVisibility("codex", false);
    await usageDashboardApi.deleteAgentModule("custom-research");

    expect(invokeMock.mock.calls).toEqual([
      ["list_dashboard_modules"],
      ["save_dashboard_module", { input: agentInput }],
      ["reorder_dashboard_modules", { moduleIds: ["codex", "claude-code"] }],
      [
        "set_dashboard_module_visibility",
        { moduleId: "codex", visible: false },
      ],
      ["delete_dashboard_module", { moduleId: "custom-research" }],
    ]);
  });

  it("invokes every binding, protected-key, setup, and diagnostics command exactly", async () => {
    await usageDashboardApi.listAgentProviderBindings("codex");
    await usageDashboardApi.saveAgentProviderBinding(bindingInput);
    await usageDashboardApi.deleteAgentProviderBinding("binding-a", 2);
    await usageDashboardApi.setAgentProviderBindingApiKey(
      "binding-a",
      2,
      "set-test-key",
    );
    await usageDashboardApi.replaceAgentProviderBindingApiKey(
      "binding-a",
      3,
      "replace-test-key",
    );
    await usageDashboardApi.clearAgentProviderBindingApiKey("binding-a", 4);
    await usageDashboardApi.setSystemProviderApiKey(
      "system-openrouter-api",
      0,
      "provider-set-key",
    );
    await usageDashboardApi.replaceSystemProviderApiKey(
      "system-openrouter-api",
      1,
      "provider-replace-key",
    );
    await usageDashboardApi.clearSystemProviderApiKey(
      "system-openrouter-api",
      2,
    );
    await usageDashboardApi.testSystemProviderConnection(
      "system-openrouter-api",
      2,
    );
    await usageDashboardApi.revealAgentProviderLocalKey("binding-a", 4);
    await usageDashboardApi.rotateAgentProviderLocalKey("binding-a", 4);
    await usageDashboardApi.getAgentProxySetupInfo("codex");
    await usageDashboardApi.getUnassignedUsageDiagnostics();

    expect(invokeMock.mock.calls).toEqual([
      ["list_agent_provider_bindings", { agentModuleId: "codex" }],
      ["save_agent_provider_binding", { input: bindingInput }],
      [
        "delete_agent_provider_binding",
        { bindingId: "binding-a", expectedVersion: 2 },
      ],
      [
        "set_agent_provider_binding_api_key",
        {
          bindingId: "binding-a",
          expectedVersion: 2,
          apiKey: "set-test-key",
        },
      ],
      [
        "replace_agent_provider_binding_api_key",
        {
          bindingId: "binding-a",
          expectedVersion: 3,
          apiKey: "replace-test-key",
        },
      ],
      [
        "clear_agent_provider_binding_api_key",
        { bindingId: "binding-a", expectedVersion: 4 },
      ],
      [
        "set_system_provider_api_key",
        {
          providerId: "system-openrouter-api",
          expectedVersion: 0,
          apiKey: "provider-set-key",
        },
      ],
      [
        "replace_system_provider_api_key",
        {
          providerId: "system-openrouter-api",
          expectedVersion: 1,
          apiKey: "provider-replace-key",
        },
      ],
      [
        "clear_system_provider_api_key",
        { providerId: "system-openrouter-api", expectedVersion: 2 },
      ],
      [
        "test_system_provider_connection",
        { providerId: "system-openrouter-api", expectedVersion: 2 },
      ],
      [
        "reveal_agent_provider_local_key",
        { bindingId: "binding-a", expectedVersion: 4 },
      ],
      [
        "rotate_agent_provider_local_key",
        { bindingId: "binding-a", expectedVersion: 4 },
      ],
      ["get_agent_proxy_setup_info", { agentModuleId: "codex" }],
      ["get_unassigned_usage_diagnostics"],
    ]);
  });

  it("uses only the three official Claude CLI auth commands", async () => {
    await claudeCliAuthApi.getStatus();
    await claudeCliAuthApi.startLogin();
    await claudeCliAuthApi.logout();
    expect(invokeMock.mock.calls).toEqual([
      ["get_claude_cli_auth_status"],
      ["start_claude_cli_login"],
      ["logout_claude_cli"],
    ]);
  });

  it("keeps Provider saves independent from Agent binding membership", async () => {
    await usageDashboardApi.saveProvider(providerInput);
    expect(invokeMock).toHaveBeenCalledWith("save_usage_provider", {
      input: providerInput,
    });
    expect(providerInput).not.toHaveProperty("dashboardModuleId");
    expect(providerInput).not.toHaveProperty("quotaConfig");
  });
});

describe("Agent-scoped usage query keys", () => {
  it("includes Agent, optional Provider, range, and pagination dimensions", () => {
    expect(usageDashboardKeys.dashboard("codex", 10, 20)).toEqual([
      "usage-dashboard",
      "dashboard",
      "codex",
      10,
      20,
    ]);
    expect(
      usageDashboardKeys.events("codex", "provider-a", 10, 20, 2, 50),
    ).toEqual([
      "usage-dashboard",
      "events",
      "codex",
      "provider-a",
      10,
      20,
      2,
      50,
    ]);
    expect(
      usageDashboardKeys.events("claude-code", undefined, 10, 20, 1, 25),
    ).not.toEqual(usageDashboardKeys.events("codex", undefined, 10, 20, 1, 25));
  });
});

function wrapper(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("Agent cache isolation", () => {
  beforeEach(() => invokeMock.mockReset());

  it("rejects a dashboard response owned by another Agent", async () => {
    invokeMock.mockResolvedValue({
      agentModuleId: "claude-code",
      startAt: 10,
      endAt: 20,
      productGroups: [],
      warnings: [],
    });
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const dashboard = renderHook(() => useUsageDashboard("codex", 10, 20), {
      wrapper: wrapper(client),
    });

    await waitFor(() => expect(dashboard.result.current.isError).toBe(true));
    expect(dashboard.result.current.data).toBeUndefined();
  });

  it("rejects event rows owned by another Agent", async () => {
    invokeMock.mockResolvedValue({
      items: [{ eventId: "wrong", agentModuleId: "claude-code" }],
      total: 1,
      page: 1,
      pageSize: 5,
    });
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const events = renderHook(
      () => useUsageEvents("codex", "provider-a", 10, 20, 1, 5),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(events.result.current.isError).toBe(true));
    expect(events.result.current.data).toBeUndefined();
  });
});

describe("usage dashboard mutation invalidation", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({}));

  it("invalidates the global root after every successful normal mutation", async () => {
    const client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const saveAgent = renderHook(() => useSaveAgentModule(), {
      wrapper: wrapper(client),
    });
    const reorder = renderHook(() => useReorderAgentModules(), {
      wrapper: wrapper(client),
    });
    const visibility = renderHook(() => useSetAgentModuleVisibility(), {
      wrapper: wrapper(client),
    });
    const deleteAgent = renderHook(() => useDeleteAgentModule(), {
      wrapper: wrapper(client),
    });
    const saveProvider = renderHook(() => useSaveUsageProvider(), {
      wrapper: wrapper(client),
    });
    const providerEnabled = renderHook(() => useSetUsageProviderEnabled(), {
      wrapper: wrapper(client),
    });
    const saveBinding = renderHook(() => useSaveAgentProviderBinding(), {
      wrapper: wrapper(client),
    });
    const deleteBinding = renderHook(() => useDeleteAgentProviderBinding(), {
      wrapper: wrapper(client),
    });
    const quota = renderHook(() => useRefreshProviderQuota(), {
      wrapper: wrapper(client),
    });
    const session = renderHook(() => useSyncProviderSessionUsage(), {
      wrapper: wrapper(client),
    });

    await act(async () => saveAgent.result.current.mutateAsync(agentInput));
    await act(async () =>
      reorder.result.current.mutateAsync(["codex", "claude-code"]),
    );
    await act(async () =>
      visibility.result.current.mutateAsync({
        agentModuleId: "codex",
        visible: false,
      }),
    );
    await act(async () =>
      deleteAgent.result.current.mutateAsync("custom-research"),
    );
    await act(async () =>
      saveProvider.result.current.mutateAsync(providerInput),
    );
    await act(async () =>
      providerEnabled.result.current.mutateAsync({
        providerId: "provider-a",
        enabled: false,
      }),
    );
    await act(async () => saveBinding.result.current.mutateAsync(bindingInput));
    await act(async () =>
      deleteBinding.result.current.mutateAsync({
        bindingId: "binding-a",
        expectedVersion: 2,
      }),
    );
    await act(async () => quota.result.current.mutateAsync("provider-a"));
    await act(async () => session.result.current.mutateAsync("provider-a"));

    expect(invalidate).toHaveBeenCalledTimes(10);
    for (const [options] of invalidate.mock.calls) {
      expect(options).toEqual({ queryKey: usageDashboardKeys.all });
    }
  });

  it("keeps protected keys out of MutationCache and invalidates root on success", async () => {
    const client = new QueryClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const credential = renderHook(
      () => useAgentProviderBindingCredentialActions(),
      { wrapper: wrapper(client) },
    );

    await act(async () =>
      credential.result.current.setApiKey("binding-a", 2, "transient-test-key"),
    );

    expect(client.getMutationCache().getAll()).toHaveLength(0);
    expect(client.getQueryCache().getAll()).toHaveLength(0);
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.all,
    });
  });

  it("invalidates the root and preserves a protected-key conflict error", async () => {
    const conflict = {
      code: "credential_conflict",
      message: "The credential changed before this request completed.",
    };
    invokeMock.mockRejectedValueOnce(conflict);
    const client = new QueryClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const credential = renderHook(
      () => useAgentProviderBindingCredentialActions(),
      { wrapper: wrapper(client) },
    );

    await act(async () => {
      await expect(
        credential.result.current.clearApiKey("binding-a", 7),
      ).rejects.toBe(conflict);
    });

    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.all,
    });
  });
});
