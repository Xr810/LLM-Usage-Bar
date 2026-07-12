import { http, HttpResponse } from "msw";
import type { AppId } from "@/lib/api/types";
import type { McpServer, Provider, Settings } from "@/types";
import {
  addProvider,
  deleteProvider,
  deleteSession,
  getCurrentProviderId,
  getLiveProviderIds,
  getSessionMessages,
  getProviders,
  listProviders,
  listSessions,
  resetProviderState,
  setCurrentProviderId,
  updateProvider,
  updateSortOrder,
  getSettings,
  setSettings,
  getAppConfigDirOverride,
  setAppConfigDirOverrideState,
  getMcpConfig,
  setMcpServerEnabled,
  upsertMcpServer,
  deleteMcpServer,
} from "./state";

const TAURI_ENDPOINT = "http://tauri.local";

const withJson = async <T>(request: Request): Promise<T> => {
  try {
    const body = await request.text();
    if (!body) return {} as T;
    return JSON.parse(body) as T;
  } catch {
    return {} as T;
  }
};

const success = <T>(payload: T) => HttpResponse.json(payload as any);

const dashboardModulesFixture = [
  {
    id: "module-subscription",
    name: "Personal usage",
    kind: "subscription",
    sortOrder: 1,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
  {
    id: "module-claude",
    name: "Claude Code",
    kind: "subscription",
    sortOrder: 2,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
  {
    id: "module-kimi",
    name: "Kimi Coding Plan",
    kind: "subscription",
    sortOrder: 3,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
  {
    id: "module-api",
    name: "Metered usage",
    kind: "api",
    sortOrder: 4,
    visible: true,
    isSystem: true,
    providerCount: 2,
  },
  {
    id: "module-renamed",
    name: "Renamed research plan",
    kind: "subscription",
    sortOrder: 5,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
] as const;

const usageProvidersFixture = [
  {
    id: "subscription-official",
    name: "Official Subscription",
    billingKind: "subscription",
    productGroupId: "codex",
    tokenSources: ["session_log"],
    sessionSourceBindings: ["codex"],
    quotaSource: "codex",
    quotaIntervalSeconds: 300,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    dashboardModuleId: "module-subscription",
  },
  {
    id: "subscription-claude",
    name: "Claude Team",
    billingKind: "subscription",
    productGroupId: "claude",
    tokenSources: ["session_log"],
    sessionSourceBindings: ["claude"],
    quotaSource: "claude",
    quotaIntervalSeconds: 300,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 2,
    updatedAt: 2,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    dashboardModuleId: "module-claude",
  },
  {
    id: "subscription-kimi",
    name: "Kimi Team",
    billingKind: "subscription",
    productGroupId: "kimi",
    tokenSources: ["session_log"],
    sessionSourceBindings: [],
    quotaSource: "coding_plan",
    quotaIntervalSeconds: 300,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 3,
    updatedAt: 3,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    dashboardModuleId: "module-kimi",
  },
  {
    id: "subscription-research",
    name: "Research Subscription",
    billingKind: "subscription",
    productGroupId: "research",
    tokenSources: ["session_log"],
    sessionSourceBindings: [],
    quotaSource: null,
    quotaIntervalSeconds: 0,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 4,
    updatedAt: 4,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    dashboardModuleId: "module-renamed",
  },
  {
    id: "azure-api",
    name: "Azure API",
    billingKind: "metered",
    productGroupId: "claude",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    quotaSource: null,
    quotaIntervalSeconds: null,
    routeAppType: "claude",
    enabled: true,
    needsReview: false,
    createdAt: 5,
    updatedAt: 5,
    routeBaseUrl: "https://azure.example.com",
    hasRouteCredentials: true,
    dashboardModuleId: null,
  },
  {
    id: "openrouter-api",
    name: "OpenRouter",
    billingKind: "metered",
    productGroupId: "research",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    quotaSource: null,
    quotaIntervalSeconds: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 6,
    updatedAt: 6,
    routeBaseUrl: "https://openrouter.ai/api/v1",
    hasRouteCredentials: true,
    dashboardModuleId: null,
  },
] as const;

const usageProvider = (id: string) => {
  const provider = usageProvidersFixture.find(
    (candidate) => candidate.id === id,
  );
  if (!provider) throw new Error(`Missing usage Provider fixture: ${id}`);
  return provider;
};

export const handlers = [
  http.post(`${TAURI_ENDPOINT}/list_dashboard_modules`, () =>
    success(dashboardModulesFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/list_usage_providers`, () =>
    success(usageProvidersFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/get_route_bindings`, () =>
    success([
      { protocol: "claude", providerId: "azure-api", updatedAt: 5 },
      { protocol: "codex", providerId: "openrouter-api", updatedAt: 6 },
    ]),
  ),
  http.post(`${TAURI_ENDPOINT}/get_usage_dashboard`, async ({ request }) => {
    const { startAt, endAt } = await withJson<{
      startAt: number;
      endAt: number;
    }>(request);
    return success({
      startAt,
      endAt,
      warnings: [],
      productGroups: [
        {
          productGroupId: "codex",
          inputTokens: 60,
          outputTokens: 10,
          cacheReadTokens: 0,
          cacheCreationTokens: 0,
          totalCostUsd: null,
          costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
          tokenSources: ["session_log"],
          subscriptionProviders: [
            {
              provider: usageProvider("subscription-official"),
              eventCount: 1,
              inputTokens: 60,
              outputTokens: 10,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
              quota: {
                snapshotId: "quota-1",
                fetchedAt: endAt,
                fiveHourUtilizationPercent: "25",
                fiveHourResetsAt: null,
                sevenDayUtilizationPercent: "40",
                sevenDayResetsAt: null,
                manualResetsRemaining: 1,
              },
              quotaFetchState: null,
            },
          ],
          meteredProviders: [],
        },
        {
          productGroupId: "claude",
          inputTokens: 75,
          outputTokens: 20,
          cacheReadTokens: 5,
          cacheCreationTokens: 0,
          totalCostUsd: "1.25",
          costSourceCounts: { upstream: 1, estimated: 0, unavailable: 1 },
          tokenSources: ["session_log", "proxy"],
          subscriptionProviders: [
            {
              provider: usageProvider("subscription-claude"),
              eventCount: 1,
              inputTokens: 25,
              outputTokens: 10,
              cacheReadTokens: 5,
              cacheCreationTokens: 0,
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
              quota: {
                snapshotId: "quota-claude",
                fetchedAt: endAt,
                fiveHourUtilizationPercent: "35",
                fiveHourResetsAt: null,
                sevenDayUtilizationPercent: "55",
                sevenDayResetsAt: null,
                manualResetsRemaining: null,
              },
              quotaFetchState: null,
            },
          ],
          meteredProviders: [
            {
              provider: usageProvider("azure-api"),
              eventCount: 1,
              inputTokens: 50,
              outputTokens: 10,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              totalCostUsd: "1.25",
              costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
              quota: null,
              quotaFetchState: null,
            },
          ],
        },
        {
          productGroupId: "kimi",
          inputTokens: 30,
          outputTokens: 5,
          cacheReadTokens: 0,
          cacheCreationTokens: 0,
          totalCostUsd: null,
          costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
          tokenSources: ["session_log"],
          subscriptionProviders: [
            {
              provider: usageProvider("subscription-kimi"),
              eventCount: 1,
              inputTokens: 30,
              outputTokens: 5,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
              quota: {
                snapshotId: "quota-kimi",
                fetchedAt: endAt,
                fiveHourUtilizationPercent: "15",
                fiveHourResetsAt: null,
                sevenDayUtilizationPercent: "20",
                sevenDayResetsAt: null,
                manualResetsRemaining: 2,
              },
              quotaFetchState: null,
            },
          ],
          meteredProviders: [],
        },
        {
          productGroupId: "research",
          inputTokens: 35,
          outputTokens: 10,
          cacheReadTokens: 0,
          cacheCreationTokens: 0,
          totalCostUsd: null,
          costSourceCounts: { upstream: 0, estimated: 0, unavailable: 2 },
          tokenSources: ["session_log", "proxy"],
          subscriptionProviders: [
            {
              provider: usageProvider("subscription-research"),
              eventCount: 1,
              inputTokens: 15,
              outputTokens: 5,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
              quota: null,
              quotaFetchState: null,
            },
          ],
          meteredProviders: [
            {
              provider: usageProvider("openrouter-api"),
              eventCount: 1,
              inputTokens: 20,
              outputTokens: 5,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              totalCostUsd: null,
              costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
              quota: null,
              quotaFetchState: null,
            },
          ],
        },
      ],
    });
  }),
  http.post(`${TAURI_ENDPOINT}/get_usage_events`, async ({ request }) => {
    const { providerId } = await withJson<{ providerId: string }>(request);
    const event =
      providerId === "azure-api"
        ? {
            eventId: "event-azure",
            source: "proxy",
            providerId,
            productGroupId: "claude",
            occurredAt: 5,
            model: "claude-sonnet-4",
            inputTokens: 50,
            outputTokens: 10,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            requestId: "request-azure",
            sessionId: null,
            upstreamCorrelationId: null,
            inputCostUsd: "1.00",
            outputCostUsd: "0.25",
            cacheReadCostUsd: null,
            cacheCreationCostUsd: null,
            totalCostUsd: "1.25",
            costSource: "upstream",
            legacyRequestId: null,
            createdAt: 5,
          }
        : providerId === "openrouter-api"
          ? {
              eventId: "event-openrouter",
              source: "proxy",
              providerId,
              productGroupId: "research",
              occurredAt: 6,
              model: "gpt-4.1",
              inputTokens: 20,
              outputTokens: 5,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              requestId: "request-openrouter",
              sessionId: null,
              upstreamCorrelationId: null,
              inputCostUsd: null,
              outputCostUsd: null,
              cacheReadCostUsd: null,
              cacheCreationCostUsd: null,
              totalCostUsd: null,
              costSource: "unavailable",
              legacyRequestId: null,
              createdAt: 6,
            }
          : null;
    return success({
      items: event ? [event] : [],
      total: event ? 1 : 0,
      page: 1,
      pageSize: 5,
    });
  }),
  http.post(`${TAURI_ENDPOINT}/is_proxy_running`, () => success(false)),
  http.post(`${TAURI_ENDPOINT}/get_migration_result`, () => success(false)),
  http.post(`${TAURI_ENDPOINT}/get_skills_migration_result`, () =>
    success(null),
  ),
  http.post(`${TAURI_ENDPOINT}/get_providers`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(getProviders(app));
  }),

  http.post(`${TAURI_ENDPOINT}/get_current_provider`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(getCurrentProviderId(app));
  }),

  http.post(
    `${TAURI_ENDPOINT}/update_providers_sort_order`,
    async ({ request }) => {
      const { updates = [], app } = await withJson<{
        updates: { id: string; sortIndex: number }[];
        app: AppId;
      }>(request);
      updateSortOrder(app, updates);
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/update_tray_menu`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_opencode_live_provider_ids`, () =>
    success(getLiveProviderIds("opencode")),
  ),

  http.post(`${TAURI_ENDPOINT}/get_openclaw_live_provider_ids`, () =>
    success(getLiveProviderIds("openclaw")),
  ),

  http.post(`${TAURI_ENDPOINT}/get_openclaw_default_model`, () =>
    success({ primary: null, fallback: [] }),
  ),

  http.post(`${TAURI_ENDPOINT}/scan_openclaw_config_health`, () => success([])),

  http.post(`${TAURI_ENDPOINT}/switch_provider`, async ({ request }) => {
    const { id, app } = await withJson<{ id: string; app: AppId }>(request);
    const providers = listProviders(app);
    if (!providers[id]) {
      return HttpResponse.json(false, { status: 404 });
    }
    setCurrentProviderId(app, id);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/add_provider`, async ({ request }) => {
    const { provider, app } = await withJson<{
      provider: Provider & { id?: string };
      app: AppId;
    }>(request);

    const newId = provider.id ?? `mock-${Date.now()}`;
    addProvider(app, { ...provider, id: newId });
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/update_provider`, async ({ request }) => {
    const { provider, app } = await withJson<{
      provider: Provider;
      app: AppId;
    }>(request);
    updateProvider(app, provider);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/delete_provider`, async ({ request }) => {
    const { id, app } = await withJson<{ id: string; app: AppId }>(request);
    deleteProvider(app, id);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/import_default_config`, async () => {
    resetProviderState();
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/open_external`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/list_sessions`, () => success(listSessions())),

  http.post(`${TAURI_ENDPOINT}/get_session_messages`, async ({ request }) => {
    const { providerId, sourcePath } = await withJson<{
      providerId: string;
      sourcePath: string;
    }>(request);
    return success(getSessionMessages(providerId, sourcePath));
  }),

  http.post(`${TAURI_ENDPOINT}/delete_session`, async ({ request }) => {
    const { providerId, sessionId, sourcePath } = await withJson<{
      providerId: string;
      sessionId: string;
      sourcePath: string;
    }>(request);
    return success(deleteSession(providerId, sessionId, sourcePath));
  }),

  http.post(`${TAURI_ENDPOINT}/delete_sessions`, async ({ request }) => {
    const { items = [] } = await withJson<{
      items?: {
        providerId: string;
        sessionId: string;
        sourcePath: string;
      }[];
    }>(request);

    return success(
      items.map((item) => ({
        providerId: item.providerId,
        sessionId: item.sessionId,
        sourcePath: item.sourcePath,
        success: deleteSession(
          item.providerId,
          item.sessionId,
          item.sourcePath,
        ),
      })),
    );
  }),

  // MCP APIs
  http.post(`${TAURI_ENDPOINT}/get_mcp_config`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(getMcpConfig(app));
  }),

  http.post(`${TAURI_ENDPOINT}/import_mcp_from_claude`, () => success(1)),
  http.post(`${TAURI_ENDPOINT}/import_mcp_from_codex`, () => success(1)),

  http.post(`${TAURI_ENDPOINT}/set_mcp_enabled`, async ({ request }) => {
    const { app, id, enabled } = await withJson<{
      app: AppId;
      id: string;
      enabled: boolean;
    }>(request);
    setMcpServerEnabled(app, id, enabled);
    return success(true);
  }),

  http.post(
    `${TAURI_ENDPOINT}/upsert_mcp_server_in_config`,
    async ({ request }) => {
      const { app, id, spec } = await withJson<{
        app: AppId;
        id: string;
        spec: McpServer;
      }>(request);
      upsertMcpServer(app, id, spec);
      return success(true);
    },
  ),

  http.post(
    `${TAURI_ENDPOINT}/delete_mcp_server_in_config`,
    async ({ request }) => {
      const { app, id } = await withJson<{ app: AppId; id: string }>(request);
      deleteMcpServer(app, id);
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/restart_app`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_settings`, () => success(getSettings())),

  http.post(`${TAURI_ENDPOINT}/check_env_conflicts`, () => success([])),

  http.post(`${TAURI_ENDPOINT}/save_settings`, async ({ request }) => {
    const { settings } = await withJson<{ settings: Settings }>(request);
    setSettings(settings);
    return success(true);
  }),

  http.post(
    `${TAURI_ENDPOINT}/set_app_config_dir_override`,
    async ({ request }) => {
      const { path } = await withJson<{ path: string | null }>(request);
      setAppConfigDirOverrideState(path ?? null);
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/get_app_config_dir_override`, () =>
    success(getAppConfigDirOverride()),
  ),

  http.post(
    `${TAURI_ENDPOINT}/apply_claude_plugin_config`,
    async ({ request }) => {
      const { official } = await withJson<{ official: boolean }>(request);
      setSettings({ enableClaudePluginIntegration: !official });
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/apply_claude_onboarding_skip`, () =>
    success(true),
  ),

  http.post(`${TAURI_ENDPOINT}/clear_claude_onboarding_skip`, () =>
    success(true),
  ),

  http.post(`${TAURI_ENDPOINT}/get_config_dir`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(app === "claude" ? "/default/claude" : "/default/codex");
  }),

  http.post(`${TAURI_ENDPOINT}/is_portable_mode`, () => success(false)),

  http.post(
    `${TAURI_ENDPOINT}/select_config_directory`,
    async ({ request }) => {
      const { defaultPath, default_path } = await withJson<{
        defaultPath?: string;
        default_path?: string;
      }>(request);
      const initial = defaultPath ?? default_path;
      return success(initial ? `${initial}/picked` : "/mock/selected-dir");
    },
  ),

  http.post(`${TAURI_ENDPOINT}/pick_directory`, async ({ request }) => {
    const { defaultPath, default_path } = await withJson<{
      defaultPath?: string;
      default_path?: string;
    }>(request);
    const initial = defaultPath ?? default_path;
    return success(initial ? `${initial}/picked` : "/mock/selected-dir");
  }),

  http.post(`${TAURI_ENDPOINT}/open_file_dialog`, () =>
    success("/mock/import-settings.json"),
  ),

  http.post(
    `${TAURI_ENDPOINT}/import_config_from_file`,
    async ({ request }) => {
      const { filePath } = await withJson<{ filePath: string }>(request);
      if (!filePath) {
        return success({ success: false, message: "Missing file" });
      }
      setSettings({ language: "en" });
      return success({ success: true, backupId: "backup-123" });
    },
  ),

  http.post(`${TAURI_ENDPOINT}/export_config_to_file`, async ({ request }) => {
    const { filePath } = await withJson<{ filePath: string }>(request);
    if (!filePath) {
      return success({ success: false, message: "Invalid destination" });
    }
    return success({ success: true, filePath });
  }),

  http.post(`${TAURI_ENDPOINT}/save_file_dialog`, () =>
    success("/mock/export-settings.json"),
  ),

  // Sync current providers live (no-op success)
  http.post(`${TAURI_ENDPOINT}/sync_current_providers_live`, () =>
    success({ success: true }),
  ),

  // Proxy status (for SettingsPage / ProxyPanel hooks)
  http.post(`${TAURI_ENDPOINT}/get_proxy_status`, () =>
    success({
      running: false,
      address: "127.0.0.1",
      port: 0,
      active_connections: 0,
      total_requests: 0,
      success_requests: 0,
      failed_requests: 0,
      success_rate: 0,
      uptime_seconds: 0,
      current_provider: null,
      current_provider_id: null,
      last_request_at: null,
      last_error: null,
      failover_count: 0,
      active_targets: [],
    }),
  ),

  http.post(`${TAURI_ENDPOINT}/get_proxy_takeover_status`, () =>
    success({
      claude: false,
      codex: false,
      gemini: false,
    }),
  ),

  http.post(`${TAURI_ENDPOINT}/is_live_takeover_active`, () => success(false)),

  // Failover / circuit breaker defaults
  http.post(`${TAURI_ENDPOINT}/get_failover_queue`, () => success([])),
  http.post(`${TAURI_ENDPOINT}/get_available_providers_for_failover`, () =>
    success([]),
  ),
  http.post(`${TAURI_ENDPOINT}/add_to_failover_queue`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/remove_from_failover_queue`, () =>
    success(true),
  ),
  http.post(`${TAURI_ENDPOINT}/reorder_failover_queue`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/set_failover_item_enabled`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_circuit_breaker_config`, () =>
    success({
      failureThreshold: 3,
      successThreshold: 2,
      timeoutSeconds: 60,
      errorRateThreshold: 50,
      minRequests: 5,
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/update_circuit_breaker_config`, () =>
    success(true),
  ),
  http.post(`${TAURI_ENDPOINT}/get_provider_health`, () =>
    success({
      provider_id: "mock-provider",
      app_type: "claude",
      is_healthy: true,
      consecutive_failures: 0,
      last_success_at: null,
      last_failure_at: null,
      last_error: null,
      updated_at: new Date().toISOString(),
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/reset_circuit_breaker`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/get_circuit_breaker_stats`, () => success(null)),
];
