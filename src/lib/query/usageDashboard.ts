import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import type {
  DashboardModuleInput,
  UsageProviderInput,
} from "@/types/usageDashboard";

export const usageDashboardKeys = {
  all: ["usage-dashboard"] as const,
  modules: () => [...usageDashboardKeys.all, "modules"] as const,
  providers: () => [...usageDashboardKeys.all, "providers"] as const,
  provider: (providerId: string) =>
    [...usageDashboardKeys.providers(), providerId] as const,
  bindings: () => [...usageDashboardKeys.all, "bindings"] as const,
  dashboards: () => [...usageDashboardKeys.all, "dashboard"] as const,
  dashboard: (startAt: number, endAt: number, productGroupId?: string) =>
    [
      ...usageDashboardKeys.dashboards(),
      startAt,
      endAt,
      productGroupId ?? null,
    ] as const,
  eventsRoot: (providerId: string) =>
    [...usageDashboardKeys.all, "events", providerId] as const,
  eventsAll: () => [...usageDashboardKeys.all, "events"] as const,
  events: (
    providerId: string,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ) =>
    [
      ...usageDashboardKeys.eventsRoot(providerId),
      startAt,
      endAt,
      page,
      pageSize,
    ] as const,
};

export function useDashboardModules() {
  return useQuery({
    queryKey: usageDashboardKeys.modules(),
    queryFn: usageDashboardApi.listDashboardModules,
  });
}

export function useUsageProviders() {
  return useQuery({
    queryKey: usageDashboardKeys.providers(),
    queryFn: usageDashboardApi.listProviders,
  });
}

export function useRouteBindings() {
  return useQuery({
    queryKey: usageDashboardKeys.bindings(),
    queryFn: usageDashboardApi.getRouteBindings,
  });
}

export function useUsageDashboard(
  startAt: number,
  endAt: number,
  productGroupId?: string,
) {
  return useQuery({
    queryKey: usageDashboardKeys.dashboard(startAt, endAt, productGroupId),
    queryFn: () =>
      usageDashboardApi.getDashboard(startAt, endAt, productGroupId),
    enabled: startAt < endAt,
  });
}

export function useUsageEvents(
  providerId: string,
  startAt: number,
  endAt: number,
  page: number,
  pageSize: number,
) {
  return useQuery({
    queryKey: usageDashboardKeys.events(
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    ),
    queryFn: () =>
      usageDashboardApi.getEvents(providerId, startAt, endAt, page, pageSize),
    enabled: Boolean(providerId) && startAt < endAt,
  });
}

function useInvalidateConfiguration() {
  const queryClient = useQueryClient();
  return () => {
    queryClient.invalidateQueries({ queryKey: usageDashboardKeys.modules() });
    queryClient.invalidateQueries({ queryKey: usageDashboardKeys.providers() });
    queryClient.invalidateQueries({ queryKey: usageDashboardKeys.bindings() });
    queryClient.invalidateQueries({
      queryKey: usageDashboardKeys.dashboards(),
    });
  };
}

export function useSaveDashboardModule() {
  const invalidate = useInvalidateConfiguration();
  return useMutation({
    mutationFn: (input: DashboardModuleInput) =>
      usageDashboardApi.saveDashboardModule(input),
    onSuccess: invalidate,
  });
}

export function useReorderDashboardModules() {
  const invalidate = useInvalidateConfiguration();
  return useMutation({
    mutationFn: (moduleIds: string[]) =>
      usageDashboardApi.reorderDashboardModules(moduleIds),
    onSuccess: invalidate,
  });
}

export function useSetDashboardModuleVisibility() {
  const invalidate = useInvalidateConfiguration();
  return useMutation({
    mutationFn: ({
      moduleId,
      visible,
    }: {
      moduleId: string;
      visible: boolean;
    }) => usageDashboardApi.setDashboardModuleVisibility(moduleId, visible),
    onSuccess: invalidate,
  });
}

export function useDeleteDashboardModule() {
  const invalidate = useInvalidateConfiguration();
  return useMutation({
    mutationFn: (moduleId: string) =>
      usageDashboardApi.deleteDashboardModule(moduleId),
    onSuccess: invalidate,
  });
}

export function useSaveUsageProvider() {
  const invalidate = useInvalidateConfiguration();
  return useMutation({
    mutationFn: (input: UsageProviderInput) =>
      usageDashboardApi.saveProvider(input),
    onSuccess: invalidate,
  });
}

export function useSetUsageProviderEnabled() {
  const invalidate = useInvalidateConfiguration();
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

export function useSetRouteBinding() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      protocol,
      providerId,
    }: {
      protocol: string;
      providerId: string;
    }) => usageDashboardApi.setRouteBinding(protocol, providerId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.providers(),
      });
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.bindings(),
      });
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.dashboards(),
      });
    },
  });
}

export function useRefreshProviderQuota() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (providerId: string) =>
      usageDashboardApi.refreshQuota(providerId),
    onSuccess: (_, providerId) => {
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.provider(providerId),
      });
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.dashboards(),
      });
    },
  });
}

export function useSyncProviderSessionUsage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (providerId: string) =>
      usageDashboardApi.syncSessionUsage(providerId),
    onSuccess: (_, providerId) => {
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.provider(providerId),
      });
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.dashboards(),
      });
      queryClient.invalidateQueries({
        queryKey: usageDashboardKeys.eventsRoot(providerId),
      });
    },
  });
}
