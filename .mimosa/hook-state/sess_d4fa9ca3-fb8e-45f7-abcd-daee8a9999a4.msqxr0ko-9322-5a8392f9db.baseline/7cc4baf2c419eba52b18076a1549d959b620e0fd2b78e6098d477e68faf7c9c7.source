import { useQuery } from "@tanstack/react-query";
import { providerUsageActivityApi } from "@/lib/api/providerUsageActivity";

const ACTIVITY_REFRESH_MS = 5 * 60 * 1_000;

export const providerUsageActivityKeys = {
  all: ["provider-usage-activity"] as const,
  range: (startAt: number, endAt: number) =>
    [...providerUsageActivityKeys.all, startAt, endAt] as const,
};

export function useProviderUsageActivity(startAt: number, endAt: number) {
  return useQuery({
    queryKey: providerUsageActivityKeys.range(startAt, endAt),
    queryFn: () => providerUsageActivityApi.getActivity(startAt, endAt),
    enabled: startAt < endAt,
    refetchInterval: ACTIVITY_REFRESH_MS,
    staleTime: 60_000,
  });
}
