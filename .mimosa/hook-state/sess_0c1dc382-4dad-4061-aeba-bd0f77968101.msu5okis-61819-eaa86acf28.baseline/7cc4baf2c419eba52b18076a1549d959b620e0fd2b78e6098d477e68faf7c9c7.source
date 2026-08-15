import { useQuery } from "@tanstack/react-query";
import { providerUsageActivityApi } from "@/lib/api/providerUsageActivity";

export const providerUsageActivityKeys = {
  all: ["provider-usage-activity"] as const,
  range: (startAt: number, endAt: number) =>
    [...providerUsageActivityKeys.all, startAt, endAt] as const,
};

/**
 * Activity data changes only when new usage is recorded, and the backend already
 * announces that (`usage-dashboard-invalidated`, wired in `useUsageEventBridge`).
 *
 * So there is deliberately no `refetchInterval` here. A timer would fire on its own
 * schedule whether or not anything changed — and because closing the window only
 * hides it (minimize-to-tray), the webview stays alive and the timer kept polling
 * a dashboard nobody could see. Event-driven means zero queries while idle and
 * zero while the window is away.
 *
 * `staleTime` still earns its keep: it suppresses a redundant fetch when the view
 * remounts moments after the last one.
 */
export function useProviderUsageActivity(startAt: number, endAt: number) {
  return useQuery({
    queryKey: providerUsageActivityKeys.range(startAt, endAt),
    queryFn: () => providerUsageActivityApi.getActivity(startAt, endAt),
    enabled: startAt < endAt,
    staleTime: 60_000,
  });
}
