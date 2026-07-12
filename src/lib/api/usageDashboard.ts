import { invoke } from "@tauri-apps/api/core";
import type {
  DashboardModuleInput,
  DashboardModuleView,
  ProviderSessionSyncResult,
  QuotaRefreshResult,
  RouteBinding,
  UsageDashboardView,
  UsageEventPage,
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";

export const usageDashboardApi = {
  listDashboardModules: (): Promise<DashboardModuleView[]> =>
    invoke("list_dashboard_modules"),
  saveDashboardModule: (
    input: DashboardModuleInput,
  ): Promise<DashboardModuleView> => invoke("save_dashboard_module", { input }),
  reorderDashboardModules: (
    moduleIds: string[],
  ): Promise<DashboardModuleView[]> =>
    invoke("reorder_dashboard_modules", { moduleIds }),
  setDashboardModuleVisibility: (
    moduleId: string,
    visible: boolean,
  ): Promise<DashboardModuleView> =>
    invoke("set_dashboard_module_visibility", { moduleId, visible }),
  deleteDashboardModule: (moduleId: string): Promise<void> =>
    invoke("delete_dashboard_module", { moduleId }),
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
