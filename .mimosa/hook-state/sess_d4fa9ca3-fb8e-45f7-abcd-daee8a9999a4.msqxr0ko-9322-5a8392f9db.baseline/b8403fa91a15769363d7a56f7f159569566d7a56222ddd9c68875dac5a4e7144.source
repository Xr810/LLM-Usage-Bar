import { invoke } from "@tauri-apps/api/core";
import type {
  AgentModuleInput,
  AgentModuleView,
  AgentProviderBindingInput,
  AgentProviderBindingView,
  AgentProxySetupInfo,
  AgentUsageBreakdownView,
  LocalBindingKeyReveal,
  ModelUsageDashboardView,
  ProviderApiKeyView,
  ProviderSessionSyncResult,
  ProviderMonitoringDashboardView,
  QuotaRefreshResult,
  UnassignedUsageDiagnostics,
  UsageDashboardView,
  UsageEventPage,
  UsageProviderInput,
  UsageProviderView,
  SystemProviderConnectionTestResult,
  SystemProviderKeyUsageView,
} from "@/types/usageDashboard";

export const usageDashboardApi = {
  listAgentModules: (): Promise<AgentModuleView[]> =>
    invoke("list_dashboard_modules"),
  saveAgentModule: (input: AgentModuleInput): Promise<AgentModuleView> =>
    invoke("save_dashboard_module", { input }),
  reorderAgentModules: (moduleIds: string[]): Promise<AgentModuleView[]> =>
    invoke("reorder_dashboard_modules", { moduleIds }),
  setAgentModuleVisibility: (
    moduleId: string,
    visible: boolean,
  ): Promise<AgentModuleView> =>
    invoke("set_dashboard_module_visibility", { moduleId, visible }),
  deleteAgentModule: (moduleId: string): Promise<void> =>
    invoke("delete_dashboard_module", { moduleId }),

  listAgentProviderBindings: (
    agentModuleId?: string,
  ): Promise<AgentProviderBindingView[]> =>
    invoke("list_agent_provider_bindings", { agentModuleId }),
  saveAgentProviderBinding: (
    input: AgentProviderBindingInput,
  ): Promise<AgentProviderBindingView> =>
    invoke("save_agent_provider_binding", { input }),
  deleteAgentProviderBinding: (
    bindingId: string,
    expectedVersion: number,
  ): Promise<void> =>
    invoke("delete_agent_provider_binding", { bindingId, expectedVersion }),
  setAgentProviderBindingApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ): Promise<AgentProviderBindingView> =>
    invoke("set_agent_provider_binding_api_key", {
      bindingId,
      expectedVersion,
      apiKey,
    }),
  replaceAgentProviderBindingApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ): Promise<AgentProviderBindingView> =>
    invoke("replace_agent_provider_binding_api_key", {
      bindingId,
      expectedVersion,
      apiKey,
    }),
  clearAgentProviderBindingApiKey: (
    bindingId: string,
    expectedVersion: number,
  ): Promise<AgentProviderBindingView> =>
    invoke("clear_agent_provider_binding_api_key", {
      bindingId,
      expectedVersion,
    }),
  listProviderApiKeys: (providerId: string): Promise<ProviderApiKeyView[]> =>
    invoke("list_provider_api_keys", { providerId }),
  createProviderApiKey: (
    providerId: string,
    label: string,
  ): Promise<ProviderApiKeyView> =>
    invoke("create_provider_api_key", { providerId, label }),
  renameProviderApiKey: (
    keyId: string,
    label: string,
  ): Promise<ProviderApiKeyView> =>
    invoke("rename_provider_api_key", { keyId, label }),
  deleteProviderApiKey: (
    keyId: string,
    expectedVersion: number,
  ): Promise<void> =>
    invoke("delete_provider_api_key", { keyId, expectedVersion }),
  setSystemProviderApiKey: (
    keyId: string,
    expectedVersion: number,
    apiKey: string,
  ): Promise<ProviderApiKeyView> =>
    invoke("set_system_provider_api_key", {
      keyId,
      expectedVersion,
      apiKey,
    }),
  replaceSystemProviderApiKey: (
    keyId: string,
    expectedVersion: number,
    apiKey: string,
  ): Promise<ProviderApiKeyView> =>
    invoke("replace_system_provider_api_key", {
      keyId,
      expectedVersion,
      apiKey,
    }),
  clearSystemProviderApiKey: (
    keyId: string,
    expectedVersion: number,
  ): Promise<ProviderApiKeyView> =>
    invoke("clear_system_provider_api_key", {
      keyId,
      expectedVersion,
    }),
  /** Model IDs the Provider's own /v1/models reports. IDs only — the backend
      strips the rest of the body before it leaves the connection client. */
  listSystemProviderModels: (
    keyId: string,
    expectedVersion: number,
  ): Promise<string[]> =>
    invoke("list_system_provider_models", { keyId, expectedVersion }),
  testSystemProviderConnection: (
    keyId: string,
    expectedVersion: number,
  ): Promise<SystemProviderConnectionTestResult> =>
    invoke("test_system_provider_connection", {
      keyId,
      expectedVersion,
    }),
  refreshSystemProviderKeyUsage: (
    keyId: string,
    expectedVersion: number,
  ): Promise<SystemProviderKeyUsageView> =>
    invoke("refresh_system_provider_key_usage", {
      keyId,
      expectedVersion,
    }),
  revealAgentProviderLocalKey: (
    bindingId: string,
    expectedVersion: number,
  ): Promise<LocalBindingKeyReveal> =>
    invoke("reveal_agent_provider_local_key", {
      bindingId,
      expectedVersion,
    }),
  rotateAgentProviderLocalKey: (
    bindingId: string,
    expectedVersion: number,
  ): Promise<LocalBindingKeyReveal> =>
    invoke("rotate_agent_provider_local_key", {
      bindingId,
      expectedVersion,
    }),
  getAgentProxySetupInfo: (
    agentModuleId: string,
  ): Promise<AgentProxySetupInfo> =>
    invoke("get_agent_proxy_setup_info", { agentModuleId }),
  getUnassignedUsageDiagnostics: (): Promise<UnassignedUsageDiagnostics> =>
    invoke("get_unassigned_usage_diagnostics"),

  listProviders: (): Promise<UsageProviderView[]> =>
    invoke("list_usage_providers"),
  saveProvider: (input: UsageProviderInput): Promise<UsageProviderView> =>
    invoke("save_usage_provider", { input }),
  deleteProvider: (providerId: string): Promise<void> =>
    invoke("delete_usage_provider", { providerId }),
  setProviderEnabled: (providerId: string, enabled: boolean): Promise<void> =>
    invoke("set_usage_provider_enabled", { providerId, enabled }),

  getDashboard: (
    agentModuleId: string,
    startAt: number,
    endAt: number,
  ): Promise<UsageDashboardView> =>
    invoke("get_usage_dashboard", { agentModuleId, startAt, endAt }),
  getProviderDashboard: (
    startAt: number,
    endAt: number,
  ): Promise<ProviderMonitoringDashboardView> =>
    invoke("get_provider_usage_dashboard", { startAt, endAt }),
  getModelDashboard: (
    startAt: number,
    endAt: number,
  ): Promise<ModelUsageDashboardView> =>
    invoke("get_model_usage_dashboard", { startAt, endAt }),
  getAgentBreakdown: (
    startAt: number,
    endAt: number,
  ): Promise<AgentUsageBreakdownView> =>
    invoke("get_agent_usage_breakdown", { startAt, endAt }),
  getEvents: (
    agentModuleId: string,
    providerId: string | undefined,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ): Promise<UsageEventPage> =>
    invoke("get_usage_events", {
      agentModuleId,
      providerId,
      startAt,
      endAt,
      page,
      pageSize,
    }),
  getProviderEvents: (
    providerId: string,
    startAt: number,
    endAt: number,
    page: number,
    pageSize: number,
  ): Promise<UsageEventPage> =>
    invoke("get_provider_usage_events", {
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
