import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { usageKeys } from "@/lib/query/usage";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";

/** Keep Agent-scoped and legacy usage views coherent with backend writes. */
export function useUsageEventBridge(onUsageRecorded?: () => void) {
  const queryClient = useQueryClient();

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    (async () => {
      const off = await listen("usage-dashboard-invalidated", () => {
        onUsageRecorded?.();
        queryClient.invalidateQueries({
          queryKey: usageDashboardKeys.all,
        });
        queryClient.invalidateQueries({ queryKey: usageKeys.all });
      });

      if (disposed) {
        off();
      } else {
        unlisten = off;
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [onUsageRecorded, queryClient]);
}
