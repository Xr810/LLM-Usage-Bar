import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getTrayUsageSnapshot, refreshTrayUsage } from "@/lib/api/trayUsage";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import type { TrayUsageSnapshot } from "@/types/trayUsage";

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

export function useTrayUsageEventBridge() {
  const queryClient = useQueryClient();

  useTauriEvent<TrayUsageSnapshot>("tray-usage-updated", (snapshot) => {
    queryClient.setQueryData(trayUsageKeys.snapshot(), snapshot);
  });
}
