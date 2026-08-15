import { invoke } from "@tauri-apps/api/core";
import type { UsageTrendBucketView } from "@/types/usageDashboard";

export const providerUsageActivityApi = {
  getActivity: (
    startAt: number,
    endAt: number,
  ): Promise<UsageTrendBucketView[]> =>
    invoke("get_provider_usage_activity", { startAt, endAt }),
};
