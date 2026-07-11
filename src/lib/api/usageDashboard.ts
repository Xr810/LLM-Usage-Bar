import { invoke } from "@tauri-apps/api/core";
import type {
  ProviderSessionSyncResult,
  QuotaRefreshResult,
  RouteBinding,
  UsageDashboardView,
  UsageEventPage,
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";

export const usageDashboardApi = {
  listProviders: (): Promise<UsageProviderView[]> =>
    invoke("list_usage_providers"),
  saveProvider: (input: UsageProviderInput): Promise<UsageProviderView> =>
    invoke("save_usage_provider", { input }),
  setProviderEnabled: (providerId: string, enabled: boolean): Promise<void> =>
    invoke("set_usage_provider_enabled", { providerId, enabled }),
  getRouteBindings: (): Promise<RouteBinding[]> => invoke("get_route_bindings"),
  setRouteBinding: (
    protocol: string,
    providerId: string,
  ): Promise<RouteBinding> =>
    invoke("set_route_binding", { protocol, providerId }),
  getDashboard: (
    startAt: number,
    endAt: number,
    productGroupId?: string,
  ): Promise<UsageDashboardView> =>
    invoke("get_usage_dashboard", {
      startAt,
      endAt,
      productGroupId,
    }),
  getEvents: (
    providerId: string,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ): Promise<UsageEventPage> =>
    invoke("get_usage_events", {
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    }),
  refreshQuota: (providerId: string): Promise<QuotaRefreshResult> =>
    invoke("refresh_provider_quota", { providerId }),
  syncSessionUsage: (providerId: string): Promise<ProviderSessionSyncResult> =>
    invoke("sync_provider_session_usage", { providerId }),
};
