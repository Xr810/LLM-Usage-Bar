import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getTrayUsageSnapshot,
  refreshTrayUsage,
  setProviderDailyBudget,
} from "@/lib/api/trayUsage";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import type { UsageProviderView } from "@/types/usageDashboard";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";

export const trayUsageKeys = {
  all: ["tray-usage"] as const,
  snapshot: () => [...trayUsageKeys.all, "snapshot"] as const,
};

export function useTrayUsageSnapshot() {
  return useQuery({
    queryKey: trayUsageKeys.snapshot(),
    queryFn: getTrayUsageSnapshot,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
    // A popover opened during the startup refresh can read the transient
    // `refreshInProgress` snapshot before its event listener is attached. Poll
    // the cache-only command until the backend clears that flag so a missed
    // terminal event cannot leave Refresh disabled for the whole window life.
    refetchInterval: (query) =>
      query.state.data?.refreshInProgress ? 500 : false,
  });
}

export function useRefreshTrayUsage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => {
      const cached = queryClient.getQueryData<TrayUsageSnapshot>(
        trayUsageKeys.snapshot(),
      );
      return cached?.refreshInProgress
        ? Promise.resolve(cached)
        : refreshTrayUsage();
    },
    onSuccess: (snapshot) => {
      queryClient.setQueryData(trayUsageKeys.snapshot(), snapshot);
    },
  });
}

export function useSetProviderDailyBudget() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      providerId,
      dailyBudgetUsd,
    }: {
      providerId: string;
      dailyBudgetUsd: string | null;
    }) => setProviderDailyBudget(providerId, dailyBudgetUsd),
    onSuccess: async (provider) => {
      queryClient.setQueryData<UsageProviderView[]>(
        usageDashboardKeys.providers(),
        (current = []) =>
          current.map((item) => (item.id === provider.id ? provider : item)),
      );
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: trayUsageKeys.all }),
        queryClient.invalidateQueries({
          queryKey: usageDashboardKeys.dashboards(),
        }),
      ]);
    },
  });
}

export function useTrayUsageEventBridge() {
  const queryClient = useQueryClient();

  useTauriEvent<TrayUsageSnapshot>("tray-usage-updated", (snapshot) => {
    queryClient.setQueryData(trayUsageKeys.snapshot(), snapshot);
  });
}
