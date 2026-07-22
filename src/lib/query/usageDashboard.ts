import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { claudeCliAuthApi } from "@/lib/api/claudeCliAuth";
import type {
  AgentModuleInput,
  AgentProviderBindingInput,
  UsageProviderInput,
} from "@/types/usageDashboard";

export const usageDashboardKeys = {
  all: ["usage-dashboard"] as const,
  agents: () => [...usageDashboardKeys.all, "agents"] as const,
  providers: () => [...usageDashboardKeys.all, "providers"] as const,
  provider: (providerId: string) =>
    [...usageDashboardKeys.providers(), providerId] as const,
  bindingsRoot: () => [...usageDashboardKeys.all, "agent-bindings"] as const,
  bindings: (agentModuleId?: string) =>
    [...usageDashboardKeys.bindingsRoot(), agentModuleId ?? null] as const,
  setupRoot: () => [...usageDashboardKeys.all, "proxy-setup"] as const,
  setup: (agentModuleId: string) =>
    [...usageDashboardKeys.setupRoot(), agentModuleId] as const,
  diagnostics: () => [...usageDashboardKeys.all, "diagnostics"] as const,
  claudeAuth: () => [...usageDashboardKeys.all, "claude-cli-auth"] as const,
  dashboards: () => [...usageDashboardKeys.all, "dashboard"] as const,
  providerDashboard: (startAt: number, endAt: number) =>
    [...usageDashboardKeys.dashboards(), "providers", startAt, endAt] as const,
  dashboard: (agentModuleId: string, startAt: number, endAt: number) =>
    [
      ...usageDashboardKeys.dashboards(),
      agentModuleId,
      startAt,
      endAt,
    ] as const,
  eventsAll: () => [...usageDashboardKeys.all, "events"] as const,
  providerEvents: (
    providerId: string,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ) =>
    [
      ...usageDashboardKeys.eventsAll(),
      "provider",
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    ] as const,
  eventsRoot: (agentModuleId: string, providerId?: string) =>
    [
      ...usageDashboardKeys.eventsAll(),
      agentModuleId,
      providerId ?? null,
    ] as const,
  events: (
    agentModuleId: string,
    providerId: string | undefined,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ) =>
    [
      ...usageDashboardKeys.eventsRoot(agentModuleId, providerId),
      startAt,
      endAt,
      page,
      pageSize,
    ] as const,
};

export function useAgentModules() {
  return useQuery({
    queryKey: usageDashboardKeys.agents(),
    queryFn: usageDashboardApi.listAgentModules,
  });
}

export function useUsageProviders() {
  return useQuery({
    queryKey: usageDashboardKeys.providers(),
    queryFn: usageDashboardApi.listProviders,
  });
}

export function useAgentProviderBindings(agentModuleId?: string) {
  return useQuery({
    queryKey: usageDashboardKeys.bindings(agentModuleId),
    queryFn: () => usageDashboardApi.listAgentProviderBindings(agentModuleId),
  });
}

export function useAgentProxySetupInfo(agentModuleId: string) {
  return useQuery({
    queryKey: usageDashboardKeys.setup(agentModuleId),
    queryFn: () => usageDashboardApi.getAgentProxySetupInfo(agentModuleId),
    enabled: Boolean(agentModuleId),
  });
}

export function useUnassignedUsageDiagnostics() {
  return useQuery({
    queryKey: usageDashboardKeys.diagnostics(),
    queryFn: usageDashboardApi.getUnassignedUsageDiagnostics,
  });
}

export function useClaudeCliAuthStatus() {
  return useQuery({
    queryKey: usageDashboardKeys.claudeAuth(),
    queryFn: claudeCliAuthApi.getStatus,
  });
}

export function useUsageDashboard(
  agentModuleId: string,
  startAt: number,
  endAt: number,
) {
  return useQuery({
    queryKey: usageDashboardKeys.dashboard(agentModuleId, startAt, endAt),
    queryFn: async () => {
      const dashboard = await usageDashboardApi.getDashboard(
        agentModuleId,
        startAt,
        endAt,
      );
      if (dashboard.agentModuleId !== agentModuleId) {
        throw new Error("usage_dashboard_agent_mismatch");
      }
      return dashboard;
    },
    enabled: Boolean(agentModuleId) && startAt < endAt,
  });
}

export function useProviderUsageDashboard(startAt: number, endAt: number) {
  return useQuery({
    queryKey: usageDashboardKeys.providerDashboard(startAt, endAt),
    queryFn: () => usageDashboardApi.getProviderDashboard(startAt, endAt),
    placeholderData: (previousData) =>
      previousData?.startAt === startAt ? previousData : undefined,
    enabled: startAt < endAt,
  });
}

export function useUsageEvents(
  agentModuleId: string,
  providerId: string | undefined,
  startAt: number,
  endAt: number,
  page: number,
  pageSize: number,
) {
  return useQuery({
    queryKey: usageDashboardKeys.events(
      agentModuleId,
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    ),
    queryFn: async () => {
      const events = await usageDashboardApi.getEvents(
        agentModuleId,
        providerId,
        startAt,
        endAt,
        page,
        pageSize,
      );
      if (events.items.some((event) => event.agentModuleId !== agentModuleId)) {
        throw new Error("usage_event_agent_mismatch");
      }
      return events;
    },
    enabled: Boolean(agentModuleId) && startAt < endAt,
  });
}

export function useProviderUsageEvents(
  providerId: string,
  startAt: number,
  endAt: number,
  page: number,
  pageSize: number,
) {
  return useQuery({
    queryKey: usageDashboardKeys.providerEvents(
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    ),
    queryFn: () =>
      usageDashboardApi.getProviderEvents(
        providerId,
        startAt,
        endAt,
        page,
        pageSize,
      ),
    enabled: Boolean(providerId) && startAt < endAt,
  });
}

function useInvalidateUsageDashboard() {
  const queryClient = useQueryClient();
  return () =>
    queryClient.invalidateQueries({ queryKey: usageDashboardKeys.all });
}

export function useSaveAgentModule() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (input: AgentModuleInput) =>
      usageDashboardApi.saveAgentModule(input),
    onSuccess: invalidate,
  });
}

export function useReorderAgentModules() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (moduleIds: string[]) =>
      usageDashboardApi.reorderAgentModules(moduleIds),
    onSuccess: invalidate,
  });
}

export function useSetAgentModuleVisibility() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: ({
      agentModuleId,
      visible,
    }: {
      agentModuleId: string;
      visible: boolean;
    }) => usageDashboardApi.setAgentModuleVisibility(agentModuleId, visible),
    onSuccess: invalidate,
  });
}

export function useDeleteAgentModule() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (agentModuleId: string) =>
      usageDashboardApi.deleteAgentModule(agentModuleId),
    onSuccess: invalidate,
  });
}

export function useSaveUsageProvider() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (input: UsageProviderInput) =>
      usageDashboardApi.saveProvider(input),
    onSuccess: invalidate,
  });
}

export function useSetUsageProviderEnabled() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: ({
      providerId,
      enabled,
    }: {
      providerId: string;
      enabled: boolean;
    }) => usageDashboardApi.setProviderEnabled(providerId, enabled),
    onSuccess: invalidate,
  });
}

export function useSaveAgentProviderBinding() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (input: AgentProviderBindingInput) =>
      usageDashboardApi.saveAgentProviderBinding(input),
    onSuccess: invalidate,
  });
}

export function useDeleteAgentProviderBinding() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: ({
      bindingId,
      expectedVersion,
    }: {
      bindingId: string;
      expectedVersion: number;
    }) =>
      usageDashboardApi.deleteAgentProviderBinding(bindingId, expectedVersion),
    onSuccess: invalidate,
  });
}

export function useAgentProviderBindingCredentialActions() {
  const queryClient = useQueryClient();
  const [isPending, setIsPending] = useState(false);
  const run = async <T>(operation: () => Promise<T>): Promise<T> => {
    setIsPending(true);
    try {
      let result: T;
      try {
        result = await operation();
      } catch (error) {
        await queryClient
          .invalidateQueries({ queryKey: usageDashboardKeys.all })
          .catch(() => undefined);
        throw error;
      }
      await queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.all,
      });
      return result;
    } finally {
      setIsPending(false);
    }
  };

  return {
    isPending,
    setApiKey: (bindingId: string, expectedVersion: number, apiKey: string) =>
      run(() =>
        usageDashboardApi.setAgentProviderBindingApiKey(
          bindingId,
          expectedVersion,
          apiKey,
        ),
      ),
    replaceApiKey: (
      bindingId: string,
      expectedVersion: number,
      apiKey: string,
    ) =>
      run(() =>
        usageDashboardApi.replaceAgentProviderBindingApiKey(
          bindingId,
          expectedVersion,
          apiKey,
        ),
      ),
    clearApiKey: (bindingId: string, expectedVersion: number) =>
      run(() =>
        usageDashboardApi.clearAgentProviderBindingApiKey(
          bindingId,
          expectedVersion,
        ),
      ),
  };
}

function useTransientUsageDashboardAction() {
  const queryClient = useQueryClient();
  const [isPending, setIsPending] = useState(false);
  const run = async <T>(operation: () => Promise<T>): Promise<T> => {
    setIsPending(true);
    try {
      return await operation();
    } finally {
      await queryClient
        .invalidateQueries({ queryKey: usageDashboardKeys.all })
        .catch(() => undefined);
      setIsPending(false);
    }
  };
  return { isPending, run };
}

export function useSystemProviderCredentialActions() {
  const { isPending, run } = useTransientUsageDashboardAction();
  return {
    isPending,
    setApiKey: (providerId: string, expectedVersion: number, apiKey: string) =>
      run(() =>
        usageDashboardApi.setSystemProviderApiKey(
          providerId,
          expectedVersion,
          apiKey,
        ),
      ),
    replaceApiKey: (
      providerId: string,
      expectedVersion: number,
      apiKey: string,
    ) =>
      run(() =>
        usageDashboardApi.replaceSystemProviderApiKey(
          providerId,
          expectedVersion,
          apiKey,
        ),
      ),
    clearApiKey: (providerId: string, expectedVersion: number) =>
      run(() =>
        usageDashboardApi.clearSystemProviderApiKey(
          providerId,
          expectedVersion,
        ),
      ),
    testConnection: (providerId: string, expectedVersion: number) =>
      run(() =>
        usageDashboardApi.testSystemProviderConnection(
          providerId,
          expectedVersion,
        ),
      ),
  };
}

export function useAgentProviderLocalKeyActions() {
  const { isPending, run } = useTransientUsageDashboardAction();
  return {
    isPending,
    reveal: (bindingId: string, expectedVersion: number) =>
      run(() =>
        usageDashboardApi.revealAgentProviderLocalKey(
          bindingId,
          expectedVersion,
        ),
      ),
    rotate: (bindingId: string, expectedVersion: number) =>
      run(() =>
        usageDashboardApi.rotateAgentProviderLocalKey(
          bindingId,
          expectedVersion,
        ),
      ),
  };
}

export function useClaudeCliAuthActions() {
  const { isPending, run } = useTransientUsageDashboardAction();
  return {
    isPending,
    startLogin: () => run(claudeCliAuthApi.startLogin),
    logout: () => run(claudeCliAuthApi.logout),
  };
}

export function useRefreshProviderQuota() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (providerId: string) =>
      usageDashboardApi.refreshQuota(providerId),
    onSuccess: invalidate,
  });
}

export function useSyncProviderSessionUsage() {
  const invalidate = useInvalidateUsageDashboard();
  return useMutation({
    mutationFn: (providerId: string) =>
      usageDashboardApi.syncSessionUsage(providerId),
    onSuccess: invalidate,
  });
}
