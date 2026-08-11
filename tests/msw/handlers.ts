import { http, HttpResponse } from "msw";
import type { AppId } from "@/lib/api/types";
import type {
  AgentModuleInput,
  AgentModuleView,
  AgentProviderBindingInput,
  AgentProviderBindingView,
  ProviderApiKeyView,
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";
import type {
  ApiBudgetConfig,
  MainWindowDestination,
  TrayUsageSnapshot,
} from "@/types/trayUsage";
const TAURI_ENDPOINT = "http://tauri.local";

export const trayUsageSnapshotFixture: TrayUsageSnapshot = {
  status: "green",
  generatedAt: 1_000,
  lastSuccessAt: 1_000,
  stale: false,
  refreshError: null,
  refreshInProgress: false,
  apiBudget: {
    mode: "per_provider",
    providerCount: 1,
    todayCostUsd: "2.5",
    dailyBudgetUsd: null,
    budgetConsumedPercent: null,
    costQuality: "complete",
    status: "green",
    warningReason: null,
  },
  agents: [
    {
      agentModuleId: "codex",
      name: "Codex",
      sortOrder: 1,
      status: "green",
      providers: [
        {
          providerId: "system-chatgpt-subscription",
          providerName: "ChatGPT",
          systemPresetKey: "chatgpt-subscription",
          billingKind: "subscription",
          status: "green",
          warningReason: null,
          recentUsage: {
            startAt: 1,
            endAt: 1_000,
            todayTokens: 0,
            totalTokens: 87_000_000,
            todayCostUsd: "1.25",
            totalCostUsd: "48.5",
            costQuality: "estimated",
            mostUsedModel: "gpt-5.6-sol",
            trendBuckets: [
              {
                startAt: 1,
                endAt: 1_000,
                eventCount: 8,
                inputTokens: 1_000_000,
                outputTokens: 200_000,
                cacheReadTokens: 3_000_000,
                cacheCreationTokens: 0,
                totalTokens: 4_200_000,
                totalCostUsd: "1.25",
                costSourceCounts: {
                  upstream: 0,
                  estimated: 8,
                  unavailable: 0,
                },
              },
            ],
          },
          subscription: {
            planLabel: "Plus",
            windows: [
              {
                kind: "five_hour",
                usedPercent: "12",
                remainingPercent: "88",
                resetsAt: "2026-07-16T12:00:00Z",
                status: "green",
                unavailableReason: null,
              },
              {
                kind: "seven_day",
                usedPercent: "30",
                remainingPercent: "70",
                resetsAt: "2026-07-20T00:00:00Z",
                status: "green",
                unavailableReason: null,
              },
            ],
          },
          metered: null,
        },
        {
          providerId: "system-openai-api",
          providerName: "OpenAI API",
          systemPresetKey: "openai-api",
          billingKind: "metered",
          status: "green",
          warningReason: null,
          recentUsage: {
            startAt: 1,
            endAt: 1_000,
            todayTokens: 0,
            totalTokens: 12_345,
            todayCostUsd: "2.5",
            totalCostUsd: "40.25",
            costQuality: "complete",
            mostUsedModel: "gpt-4.1",
            trendBuckets: [
              {
                startAt: 1,
                endAt: 1_000,
                eventCount: 3,
                inputTokens: 5_000,
                outputTokens: 345,
                cacheReadTokens: 7_000,
                cacheCreationTokens: 0,
                totalTokens: 12_345,
                totalCostUsd: "2.5",
                costSourceCounts: {
                  upstream: 3,
                  estimated: 0,
                  unavailable: 0,
                },
              },
            ],
          },
          subscription: null,
          metered: {
            todayCostUsd: "2.5",
            rolling30DayCostUsd: "40.25",
            dailyBudgetUsd: "10",
            budgetConsumedPercent: "25",
            totalTokens: 12_345,
            costQuality: "complete",
          },
        },
      ],
    },
  ],
};

const apiBudgetConfigFixture: ApiBudgetConfig = {
  mode: "shared",
  sharedDailyBudgetUsd: null,
};

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

const initialAgentModulesFixture: AgentModuleView[] = [
  {
    id: "codex",
    name: "Codex",
    sortOrder: 1,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 2,
  },
  {
    id: "claude-code",
    name: "Claude Code",
    sortOrder: 2,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 2,
  },
  {
    id: "opencode",
    name: "OpenCode",
    sortOrder: 3,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 2,
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    sortOrder: 4,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 0,
  },
  {
    id: "hermes",
    name: "Hermes",
    sortOrder: 5,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 0,
  },
  {
    id: "custom-research",
    name: "Research Agent",
    sortOrder: 6,
    visible: true,
    isFixed: false,
    archivedAt: null,
    providerCount: 2,
  },
];

const customProviderSystemFields = {
  systemPresetKey: null,
  systemAuthKind: null,
  canonicalEndpoint: null,
  compatibleAgentModuleIds: [],
  apiKeys: [],
  supportsKeyUsage: false,
  keyUsageTotal: null,
} satisfies Pick<
  UsageProviderView,
  | "systemPresetKey"
  | "systemAuthKind"
  | "canonicalEndpoint"
  | "compatibleAgentModuleIds"
  | "apiKeys"
  | "supportsKeyUsage"
  | "keyUsageTotal"
>;

const customBindingCredentialFields = {
  routeProtocol: null,
  localCredentialStatus: "not_required",
  providerCredentialStatus: "not_required",
} satisfies Pick<
  AgentProviderBindingView,
  "routeProtocol" | "localCredentialStatus" | "providerCredentialStatus"
>;

const initialUsageProvidersFixture: UsageProviderView[] = [
  {
    id: "system-chatgpt-subscription",
    name: "ChatGPT",
    billingKind: "subscription",
    productGroupId: "chatgpt-subscription",
    tokenSources: ["proxy", "session_log"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "system-binding-chatgpt-codex",
        agentModuleId: "codex",
        providerId: "system-chatgpt-subscription",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        routeProtocol: "codex",
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    quotaSource: "codex_oauth",
    quotaIntervalSeconds: 300,
    dailyBudgetUsd: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    systemPresetKey: "chatgpt-subscription",
    systemAuthKind: "codex_oauth",
    canonicalEndpoint: null,
    compatibleAgentModuleIds: ["codex"],
    apiKeys: [],
    supportsKeyUsage: false,
    keyUsageTotal: null,
  },
  {
    id: "system-claude-subscription",
    name: "Claude",
    billingKind: "subscription",
    productGroupId: "claude-subscription",
    tokenSources: ["session_log"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "system-binding-claude-code",
        agentModuleId: "claude-code",
        providerId: "system-claude-subscription",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        routeProtocol: null,
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: null,
    dailyBudgetUsd: null,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    systemPresetKey: "claude-subscription",
    systemAuthKind: "claude_cli",
    canonicalEndpoint: null,
    compatibleAgentModuleIds: ["claude-code"],
    apiKeys: [],
    supportsKeyUsage: false,
    keyUsageTotal: null,
  },
  ...(
    [
      [
        "system-openai-api",
        "OpenAI API",
        "openai-api",
        "https://api.openai.com/v1",
        ["codex", "opencode", "openclaw", "hermes"],
      ],
      [
        "system-anthropic-api",
        "Anthropic API",
        "anthropic-api",
        "https://api.anthropic.com",
        ["claude-code"],
      ],
      [
        "system-openrouter-api",
        "OpenRouter",
        "openrouter-api",
        "https://openrouter.ai/api/v1",
        ["claude-code", "codex", "opencode", "openclaw", "hermes"],
      ],
    ] as const
  ).map(
    ([
      id,
      name,
      systemPresetKey,
      canonicalEndpoint,
      compatibleAgentModuleIds,
    ]) =>
      ({
        id,
        name,
        billingKind: "metered",
        productGroupId: systemPresetKey,
        tokenSources: ["proxy"],
        sessionSourceBindings: [],
        bindings:
          id === "system-openrouter-api"
            ? compatibleAgentModuleIds
                .filter((agentModuleId) =>
                  ["opencode", "openclaw", "hermes"].includes(agentModuleId),
                )
                .map((agentModuleId, index) => ({
                  ...customBindingCredentialFields,
                  id: `system-binding-openrouter-${agentModuleId}`,
                  agentModuleId,
                  providerId: id,
                  enabled: true,
                  effectiveEnabled: false,
                  credentialStatus: "missing",
                  canClearCredential: true,
                  credentialVersion: 1,
                  routeProtocol: agentModuleId,
                  localCredentialStatus: "configured",
                  providerCredentialStatus: "missing",
                  createdAt: 2 + index,
                  updatedAt: 2 + index,
                }))
            : [],
        quotaSource: null,
        quotaIntervalSeconds: null,
        dailyBudgetUsd: null,
        routeAppType: id === "system-anthropic-api" ? "claude" : "codex",
        enabled: true,
        needsReview: false,
        createdAt: 1,
        updatedAt: 1,
        routeBaseUrl: canonicalEndpoint,
        hasRouteCredentials: false,
        systemPresetKey,
        systemAuthKind: "provider_api_key",
        canonicalEndpoint,
        compatibleAgentModuleIds: [...compatibleAgentModuleIds],
        apiKeys: [],
        supportsKeyUsage: false,
        keyUsageTotal: null,
      }) satisfies UsageProviderView,
  ),
  {
    ...customProviderSystemFields,
    id: "subscription-official",
    name: "Official Subscription",
    billingKind: "subscription",
    productGroupId: "codex",
    tokenSources: ["session_log"],
    sessionSourceBindings: ["codex"],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-codex-subscription",
        agentModuleId: "codex",
        providerId: "subscription-official",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    quotaSource: "codex",
    quotaIntervalSeconds: 300,
    dailyBudgetUsd: null,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: null,
    hasRouteCredentials: false,
  },
  {
    ...customProviderSystemFields,
    id: "subscription-claude",
    name: "Claude Team",
    billingKind: "subscription",
    productGroupId: "claude",
    tokenSources: ["session_log"],
    sessionSourceBindings: ["claude"],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-claude-subscription",
        agentModuleId: "claude-code",
        providerId: "subscription-claude",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: 2,
        updatedAt: 2,
      },
    ],
    quotaSource: "claude",
    quotaIntervalSeconds: 300,
    dailyBudgetUsd: null,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 2,
    updatedAt: 2,
    routeBaseUrl: null,
    hasRouteCredentials: false,
  },
  {
    ...customProviderSystemFields,
    id: "subscription-kimi",
    name: "Kimi Team",
    billingKind: "subscription",
    productGroupId: "kimi",
    tokenSources: ["session_log"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-opencode-subscription",
        agentModuleId: "opencode",
        providerId: "subscription-kimi",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: 3,
        updatedAt: 3,
      },
    ],
    quotaSource: "coding_plan",
    quotaIntervalSeconds: 300,
    dailyBudgetUsd: null,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 3,
    updatedAt: 3,
    routeBaseUrl: null,
    hasRouteCredentials: false,
  },
  {
    ...customProviderSystemFields,
    id: "subscription-research",
    name: "Research Subscription",
    billingKind: "subscription",
    productGroupId: "research",
    tokenSources: ["session_log"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-custom-subscription",
        agentModuleId: "custom-research",
        providerId: "subscription-research",
        enabled: true,
        effectiveEnabled: false,
        credentialStatus: "not_required",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: 4,
        updatedAt: 4,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: 0,
    dailyBudgetUsd: null,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 4,
    updatedAt: 4,
    routeBaseUrl: null,
    hasRouteCredentials: false,
  },
  {
    ...customProviderSystemFields,
    id: "azure-api",
    name: "Azure API",
    billingKind: "metered",
    productGroupId: "claude",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-codex-azure",
        agentModuleId: "codex",
        providerId: "azure-api",
        enabled: true,
        effectiveEnabled: true,
        credentialStatus: "configured",
        canClearCredential: true,
        credentialVersion: 1,
        createdAt: 5,
        updatedAt: 5,
      },
      {
        ...customBindingCredentialFields,
        id: "binding-claude-azure",
        agentModuleId: "claude-code",
        providerId: "azure-api",
        enabled: true,
        effectiveEnabled: true,
        credentialStatus: "configured",
        canClearCredential: true,
        credentialVersion: 1,
        createdAt: 5,
        updatedAt: 5,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: null,
    dailyBudgetUsd: "10",
    routeAppType: "claude",
    enabled: true,
    needsReview: false,
    createdAt: 5,
    updatedAt: 5,
    routeBaseUrl: "https://azure.example.com",
    hasRouteCredentials: true,
  },
  {
    ...customProviderSystemFields,
    id: "openrouter-api",
    name: "OpenRouter",
    billingKind: "metered",
    productGroupId: "research",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    bindings: [
      {
        ...customBindingCredentialFields,
        id: "binding-opencode-openrouter",
        agentModuleId: "opencode",
        providerId: "openrouter-api",
        enabled: true,
        effectiveEnabled: true,
        credentialStatus: "configured",
        canClearCredential: true,
        credentialVersion: 1,
        createdAt: 6,
        updatedAt: 6,
      },
      {
        ...customBindingCredentialFields,
        id: "binding-custom-openrouter",
        agentModuleId: "custom-research",
        providerId: "openrouter-api",
        enabled: false,
        effectiveEnabled: false,
        credentialStatus: "missing",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: 6,
        updatedAt: 6,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: null,
    dailyBudgetUsd: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 6,
    updatedAt: 6,
    routeBaseUrl: "https://openrouter.ai/api/v1",
    hasRouteCredentials: true,
  },
];

const cloneUsageFixture = <T>(value: T): T =>
  JSON.parse(JSON.stringify(value)) as T;

let agentModulesFixture = cloneUsageFixture(initialAgentModulesFixture);
let usageProvidersFixture = cloneUsageFixture(initialUsageProvidersFixture);

/** Locates a key and the Provider that owns it, mirroring how the backend
    resolves a key id before any credential mutation. */
function findProviderApiKey(keyId: string) {
  for (const provider of usageProvidersFixture) {
    const apiKey = provider.apiKeys.find((candidate) => candidate.id === keyId);
    if (apiKey) return { provider, apiKey };
  }
  return null;
}
let pendingMainWindowDestination: MainWindowDestination | null = null;
let nextAgentId = 1;
let nextBindingId = 1;

export const setPendingMainWindowDestination = (
  destination: MainWindowDestination | null,
) => {
  pendingMainWindowDestination = destination;
};

export const resetUsageDashboardState = () => {
  agentModulesFixture = cloneUsageFixture(initialAgentModulesFixture);
  usageProvidersFixture = cloneUsageFixture(initialUsageProvidersFixture);
  pendingMainWindowDestination = null;
  nextAgentId = 1;
  nextBindingId = 1;
  recomputeAgentBindingState();
};

const usageProvider = (id: string) => {
  const provider = usageProvidersFixture.find(
    (candidate) => candidate.id === id,
  );
  if (!provider) throw new Error(`Missing usage Provider fixture: ${id}`);
  return provider;
};

const activeAgent = (id: string) =>
  agentModulesFixture.find(
    (agent) => agent.id === id && agent.archivedAt == null,
  );

const allAgentBindings = () =>
  usageProvidersFixture.flatMap((provider) => provider.bindings);

const bindingLocation = (bindingId: string) => {
  for (const provider of usageProvidersFixture) {
    const index = provider.bindings.findIndex(
      (binding) => binding.id === bindingId,
    );
    if (index >= 0) return { provider, index };
  }
  return null;
};

const recomputeAgentBindingState = () => {
  for (const provider of usageProvidersFixture) {
    provider.bindings = provider.bindings.map((binding) => ({
      ...binding,
      effectiveEnabled:
        binding.enabled &&
        provider.enabled &&
        Boolean(activeAgent(binding.agentModuleId)) &&
        binding.credentialStatus === "configured",
    }));
  }
  for (const agent of agentModulesFixture) {
    agent.providerCount = allAgentBindings().filter(
      (binding) => binding.agentModuleId === agent.id,
    ).length;
  }
};

const sharedAccountForProvider = (providerId: string) =>
  new Set(
    usageProvider(providerId)
      .bindings.filter(
        (binding) =>
          binding.effectiveEnabled &&
          Boolean(activeAgent(binding.agentModuleId)),
      )
      .map((binding) => binding.agentModuleId),
  ).size > 1;

const rejectUsageRequest = (message: string) =>
  HttpResponse.json(message, { status: 400 });

const subscriptionUsage = (
  providerId: string,
  endAt: number,
  inputTokens: number,
) => ({
  provider: usageProvider(providerId),
  sharedAccount: sharedAccountForProvider(providerId),
  eventCount: 1,
  inputTokens,
  outputTokens: 10,
  cacheReadTokens: 5,
  cacheCreationTokens: 0,
  totalCostUsd: null,
  costSourceCounts: { upstream: 0, estimated: 0, unavailable: 1 },
  quota: {
    snapshotId: `quota-${providerId}`,
    fetchedAt: endAt,
    fiveHourUtilizationPercent: "25",
    fiveHourResetsAt: null,
    sevenDayUtilizationPercent: "40",
    sevenDayResetsAt: null,
    manualResetsRemaining: 1,
  },
  quotaFetchState: null,
});

const meteredUsage = (
  providerId: string,
  inputTokens: number,
  totalCostUsd: string | null,
) => ({
  provider: usageProvider(providerId),
  sharedAccount: sharedAccountForProvider(providerId),
  eventCount: 1,
  inputTokens,
  outputTokens: 10,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalCostUsd,
  costSourceCounts:
    totalCostUsd == null
      ? { upstream: 0, estimated: 0, unavailable: 1 }
      : { upstream: 1, estimated: 0, unavailable: 0 },
  quota: null,
  quotaFetchState: null,
});

const usageProductGroup = (
  productGroupId: string,
  subscriptionProviders: ReturnType<typeof subscriptionUsage>[],
  meteredProviders: ReturnType<typeof meteredUsage>[],
) => ({
  productGroupId,
  inputTokens: [...subscriptionProviders, ...meteredProviders].reduce(
    (sum, row) => sum + row.inputTokens,
    0,
  ),
  outputTokens: [...subscriptionProviders, ...meteredProviders].reduce(
    (sum, row) => sum + row.outputTokens,
    0,
  ),
  cacheReadTokens: [...subscriptionProviders, ...meteredProviders].reduce(
    (sum, row) => sum + row.cacheReadTokens,
    0,
  ),
  cacheCreationTokens: 0,
  totalCostUsd:
    meteredProviders.find((row) => row.totalCostUsd != null)?.totalCostUsd ??
    null,
  costSourceCounts: {
    upstream: meteredProviders.filter((row) => row.totalCostUsd != null).length,
    estimated: 0,
    unavailable:
      subscriptionProviders.length +
      meteredProviders.filter((row) => row.totalCostUsd == null).length,
  },
  tokenSources: [
    ...(subscriptionProviders.length ? ["session_log"] : []),
    ...(meteredProviders.length ? ["proxy"] : []),
  ],
  subscriptionProviders,
  meteredProviders,
});

const dashboardGroupsForAgent = (agentModuleId: string, endAt: number) => {
  switch (agentModuleId) {
    case "codex":
      return [
        usageProductGroup(
          "codex",
          [subscriptionUsage("subscription-official", endAt, 60)],
          [meteredUsage("azure-api", 50, "1.25")],
        ),
      ];
    case "claude-code":
      return [
        usageProductGroup(
          "claude",
          [subscriptionUsage("subscription-claude", endAt, 25)],
          [meteredUsage("azure-api", 35, "0.75")],
        ),
      ];
    case "opencode":
      return [
        usageProductGroup(
          "opencode",
          [subscriptionUsage("subscription-kimi", endAt, 30)],
          [meteredUsage("openrouter-api", 20, null)],
        ),
      ];
    case "custom-research":
      return [
        usageProductGroup(
          "research",
          [subscriptionUsage("subscription-research", endAt, 15)],
          // Historical rows remain visible even when the current binding is disabled.
          [meteredUsage("openrouter-api", 20, null)],
        ),
      ];
    default:
      return [];
  }
};

const noCostCounts = { upstream: 0, estimated: 0, unavailable: 0 };

/**
 * Model breakdown fixture. Opus is deliberately served by both a
 * subscription and a metered account so the merged "By model" list has a
 * row spanning two Providers.
 */
const modelUsageDashboardFixture = (startAt: number, endAt: number) => {
  const leaf = (
    model: string,
    providerId: string,
    providerName: string,
    productGroupId: string,
    billingKind: "subscription" | "metered",
    totalTokens: number,
    eventCount: number,
    totalCostUsd: string | null,
  ) => ({
    model,
    providerId,
    providerName,
    productGroupId,
    billingKind,
    eventCount,
    inputTokens: totalTokens / 2,
    outputTokens: totalTokens / 4,
    cacheReadTokens: totalTokens / 4,
    cacheCreationTokens: 0,
    totalTokens,
    totalCostUsd,
    costSourceCounts: {
      ...noCostCounts,
      upstream: totalCostUsd == null ? 0 : eventCount,
      unavailable: totalCostUsd == null ? eventCount : 0,
    },
    firstOccurredAt: startAt,
    lastOccurredAt: endAt,
  });

  const claudeModels = [
    leaf(
      "claude-opus-5",
      "system-claude-subscription",
      "Claude Pro/Max",
      "claude-subscription",
      "subscription",
      8_000,
      40,
      "4.00",
    ),
    leaf(
      "claude-sonnet-5",
      "system-claude-subscription",
      "Claude Pro/Max",
      "claude-subscription",
      "subscription",
      2_000,
      20,
      "0.50",
    ),
  ];
  const anthropicModels = [
    leaf(
      "claude-opus-5",
      "anthropic-api",
      "Anthropic API",
      "anthropic-api",
      "metered",
      3_000,
      10,
      "1.50",
    ),
  ];
  const chatgptModels = [
    leaf(
      "gpt-5.6-sol",
      "system-chatgpt-subscription",
      "ChatGPT Plus/Pro",
      "chatgpt-subscription",
      "subscription",
      4_000,
      25,
      null,
    ),
  ];

  const group = (
    productGroupId: string,
    billingKind: "subscription" | "metered",
    providerIds: string[],
    providerNames: string[],
    models: ReturnType<typeof leaf>[],
  ) => ({
    productGroupId,
    billingKind,
    providerIds,
    providerNames,
    eventCount: models.reduce((sum, row) => sum + row.eventCount, 0),
    inputTokens: models.reduce((sum, row) => sum + row.inputTokens, 0),
    outputTokens: models.reduce((sum, row) => sum + row.outputTokens, 0),
    cacheReadTokens: models.reduce((sum, row) => sum + row.cacheReadTokens, 0),
    cacheCreationTokens: 0,
    totalTokens: models.reduce((sum, row) => sum + row.totalTokens, 0),
    totalCostUsd: models.some((row) => row.totalCostUsd != null)
      ? models
          .reduce((sum, row) => sum + Number(row.totalCostUsd ?? 0), 0)
          .toFixed(2)
      : null,
    costSourceCounts: noCostCounts,
    models,
  });

  return {
    startAt,
    endAt,
    totalTokens: 17_000,
    totalEventCount: 95,
    totalCostUsd: "6.00",
    productGroups: [
      group(
        "claude-subscription",
        "subscription",
        ["system-claude-subscription"],
        ["Claude Pro/Max"],
        claudeModels,
      ),
      group(
        "chatgpt-subscription",
        "subscription",
        ["system-chatgpt-subscription"],
        ["ChatGPT Plus/Pro"],
        chatgptModels,
      ),
      group(
        "anthropic-api",
        "metered",
        ["anthropic-api"],
        ["Anthropic API"],
        anthropicModels,
      ),
    ],
    models: [
      {
        model: "claude-opus-5",
        providerIds: ["anthropic-api", "system-claude-subscription"],
        eventCount: 50,
        inputTokens: 5_500,
        outputTokens: 2_750,
        cacheReadTokens: 2_750,
        cacheCreationTokens: 0,
        totalTokens: 11_000,
        totalCostUsd: "5.50",
        costSourceCounts: { upstream: 50, estimated: 0, unavailable: 0 },
        firstOccurredAt: startAt,
        lastOccurredAt: endAt,
      },
      {
        model: "gpt-5.6-sol",
        providerIds: ["system-chatgpt-subscription"],
        eventCount: 25,
        inputTokens: 2_000,
        outputTokens: 1_000,
        cacheReadTokens: 1_000,
        cacheCreationTokens: 0,
        totalTokens: 4_000,
        totalCostUsd: null,
        costSourceCounts: { upstream: 0, estimated: 0, unavailable: 25 },
        firstOccurredAt: startAt,
        lastOccurredAt: endAt,
      },
      {
        model: "claude-sonnet-5",
        providerIds: ["system-claude-subscription"],
        eventCount: 20,
        inputTokens: 1_000,
        outputTokens: 500,
        cacheReadTokens: 500,
        cacheCreationTokens: 0,
        totalTokens: 2_000,
        totalCostUsd: "0.50",
        costSourceCounts: { upstream: 20, estimated: 0, unavailable: 0 },
        firstOccurredAt: startAt,
        lastOccurredAt: endAt,
      },
    ],
    warnings: [],
  };
};

/** Agent breakdown fixture, including the no-owner bucket sorted last. */
const agentUsageBreakdownFixture = (startAt: number, endAt: number) => {
  const agent = (
    agentModuleId: string | null,
    agentName: string | null,
    totalTokens: number,
    eventCount: number,
    totalCostUsd: string | null,
    archived = false,
  ) => ({
    agentModuleId,
    agentName,
    archived,
    visible: agentModuleId != null && !archived,
    eventCount,
    inputTokens: totalTokens / 2,
    outputTokens: totalTokens / 4,
    cacheReadTokens: totalTokens / 4,
    cacheCreationTokens: 0,
    totalTokens,
    totalCostUsd,
    costSourceCounts: {
      ...noCostCounts,
      upstream: totalCostUsd == null ? 0 : eventCount,
      unavailable: totalCostUsd == null ? eventCount : 0,
    },
    firstOccurredAt: startAt,
    lastOccurredAt: endAt,
    providers: [
      {
        providerId: "system-claude-subscription",
        providerName: "Claude Pro/Max",
        productGroupId: "claude-subscription",
        billingKind: "subscription" as const,
        eventCount,
        inputTokens: totalTokens / 2,
        outputTokens: totalTokens / 4,
        cacheReadTokens: totalTokens / 4,
        cacheCreationTokens: 0,
        totalTokens,
        totalCostUsd,
        costSourceCounts: noCostCounts,
      },
    ],
    models: [
      {
        model: "claude-opus-5",
        providerIds: ["system-claude-subscription"],
        eventCount,
        inputTokens: totalTokens / 2,
        outputTokens: totalTokens / 4,
        cacheReadTokens: totalTokens / 4,
        cacheCreationTokens: 0,
        totalTokens,
        totalCostUsd,
        costSourceCounts: noCostCounts,
        firstOccurredAt: startAt,
        lastOccurredAt: endAt,
      },
    ],
  });

  return {
    startAt,
    endAt,
    totalTokens: 17_000,
    totalEventCount: 95,
    totalCostUsd: "6.00",
    agents: [
      agent("claude-code", "Claude Code", 10_000, 60, "4.50"),
      agent("codex", "Codex", 5_000, 25, "1.50"),
      agent("legacy-agent", "Retired Agent", 1_500, 6, "0.00", true),
      agent(null, null, 500, 4, null),
    ],
    warnings: [],
  };
};

export const handlers = [
  http.post(`${TAURI_ENDPOINT}/get_tray_usage_snapshot`, () =>
    success(trayUsageSnapshotFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/refresh_tray_usage`, () =>
    success(trayUsageSnapshotFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/get_api_budget_config`, () =>
    success(apiBudgetConfigFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/set_api_budget_config`, async ({ request }) => {
    const { mode, sharedDailyBudgetUsd } =
      await withJson<ApiBudgetConfig>(request);
    const trimmed =
      typeof sharedDailyBudgetUsd === "string"
        ? sharedDailyBudgetUsd.trim()
        : null;
    if (
      (mode !== "shared" && mode !== "per_provider") ||
      (sharedDailyBudgetUsd !== null &&
        (trimmed === "" ||
          !Number.isFinite(Number(trimmed)) ||
          Number(trimmed) <= 0))
    ) {
      return rejectUsageRequest("invalid_daily_budget");
    }
    return success({
      mode,
      sharedDailyBudgetUsd: trimmed,
    } satisfies ApiBudgetConfig);
  }),
  http.post(`${TAURI_ENDPOINT}/hide_tray_popover`, () => success(null)),
  http.post(`${TAURI_ENDPOINT}/open_main_from_tray`, () => success(null)),
  http.post(`${TAURI_ENDPOINT}/take_pending_main_window_destination`, () => {
    const destination = pendingMainWindowDestination;
    pendingMainWindowDestination = null;
    return success(destination);
  }),
  http.post(`${TAURI_ENDPOINT}/quit_from_tray`, () => success(null)),
  http.post(`${TAURI_ENDPOINT}/list_dashboard_modules`, () =>
    success(agentModulesFixture),
  ),
  http.post(`${TAURI_ENDPOINT}/save_dashboard_module`, async ({ request }) => {
    const { input } = await withJson<{ input: AgentModuleInput }>(request);
    if (!input || !input.name?.trim()) {
      return rejectUsageRequest("invalid_agent_input");
    }
    if (input.id) {
      const agent = activeAgent(input.id);
      if (!agent) return rejectUsageRequest("agent_not_found");
      if (agent.isFixed && input.name.trim() !== agent.name) {
        return rejectUsageRequest("fixed_agent_immutable");
      }
      Object.assign(agent, {
        name: input.name.trim(),
        sortOrder: input.sortOrder,
        visible: input.visible,
      });
      return success(agent);
    }
    const agent: AgentModuleView = {
      id: `custom-msw-${nextAgentId++}`,
      name: input.name.trim(),
      sortOrder: input.sortOrder,
      visible: input.visible,
      isFixed: false,
      archivedAt: null,
      providerCount: 0,
    };
    agentModulesFixture.push(agent);
    return success(agent);
  }),
  http.post(
    `${TAURI_ENDPOINT}/reorder_dashboard_modules`,
    async ({ request }) => {
      const { moduleIds } = await withJson<{ moduleIds: string[] }>(request);
      const activeIds = agentModulesFixture
        .filter((agent) => agent.archivedAt == null)
        .map((agent) => agent.id);
      if (
        !Array.isArray(moduleIds) ||
        moduleIds.length !== activeIds.length ||
        activeIds.some((id) => !moduleIds.includes(id))
      ) {
        return rejectUsageRequest("invalid_agent_order");
      }
      moduleIds.forEach((id, index) => {
        const agent = activeAgent(id);
        if (agent) agent.sortOrder = index + 1;
      });
      return success(
        agentModulesFixture
          .filter((agent) => agent.archivedAt == null)
          .sort((left, right) => left.sortOrder - right.sortOrder),
      );
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/set_dashboard_module_visibility`,
    async ({ request }) => {
      const { moduleId, visible } = await withJson<{
        moduleId: string;
        visible: boolean;
      }>(request);
      const agent = activeAgent(moduleId);
      if (!agent || typeof visible !== "boolean") {
        return rejectUsageRequest("invalid_agent_visibility");
      }
      agent.visible = visible;
      return success(agent);
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/delete_dashboard_module`,
    async ({ request }) => {
      const { moduleId } = await withJson<{ moduleId: string }>(request);
      const agent = activeAgent(moduleId);
      if (!agent || agent.isFixed) {
        return rejectUsageRequest("agent_delete_not_allowed");
      }
      agent.archivedAt = 1_000;
      agent.visible = false;
      recomputeAgentBindingState();
      return success(null);
    },
  ),
  http.post(`${TAURI_ENDPOINT}/list_usage_providers`, () =>
    success(usageProvidersFixture),
  ),
  http.post(
    `${TAURI_ENDPOINT}/set_provider_daily_budget`,
    async ({ request }) => {
      const { providerId, dailyBudgetUsd } = await withJson<{
        providerId: string;
        dailyBudgetUsd: string | null;
      }>(request);
      const provider = usageProvidersFixture.find(
        (candidate) => candidate.id === providerId,
      );
      const trimmed =
        typeof dailyBudgetUsd === "string" ? dailyBudgetUsd.trim() : null;
      if (
        !provider ||
        provider.billingKind !== "metered" ||
        (dailyBudgetUsd !== null &&
          (trimmed === "" ||
            !Number.isFinite(Number(trimmed)) ||
            Number(trimmed) <= 0))
      ) {
        return rejectUsageRequest("invalid_daily_budget");
      }
      provider.dailyBudgetUsd = trimmed;
      provider.updatedAt += 1;
      return success(provider);
    },
  ),
  ...(
    ["set_system_provider_api_key", "replace_system_provider_api_key"] as const
  ).map((command) =>
    http.post(`${TAURI_ENDPOINT}/${command}`, async ({ request }) => {
      const { keyId, expectedVersion } = await withJson<{
        keyId: string;
        expectedVersion: number;
        apiKey: string;
      }>(request);
      const owner = findProviderApiKey(keyId);
      if (!owner || owner.apiKey.credentialVersion !== expectedVersion) {
        return rejectUsageRequest("credential_conflict");
      }
      owner.apiKey.credentialStatus = "configured";
      owner.apiKey.credentialVersion += 1;
      owner.apiKey.canClearCredential = true;
      owner.provider.updatedAt += 1;
      owner.provider.bindings = owner.provider.bindings.map((binding) => ({
        ...binding,
        credentialStatus:
          binding.localCredentialStatus === "configured"
            ? "configured"
            : binding.credentialStatus,
        providerCredentialStatus: "configured",
      }));
      recomputeAgentBindingState();
      return success(owner.apiKey);
    }),
  ),
  http.post(
    `${TAURI_ENDPOINT}/clear_system_provider_api_key`,
    async ({ request }) => {
      const { keyId, expectedVersion } = await withJson<{
        keyId: string;
        expectedVersion: number;
      }>(request);
      const owner = findProviderApiKey(keyId);
      if (!owner || owner.apiKey.credentialVersion !== expectedVersion) {
        return rejectUsageRequest("credential_conflict");
      }
      owner.apiKey.credentialStatus = "missing";
      owner.apiKey.credentialVersion += 1;
      owner.apiKey.canClearCredential = false;
      owner.apiKey.keyUsage = null;
      owner.provider.updatedAt += 1;
      // A Provider keeps working while any other key is still configured.
      const stillConfigured = owner.provider.apiKeys.some(
        (candidate) => candidate.credentialStatus === "configured",
      );
      owner.provider.bindings = owner.provider.bindings.map((binding) => ({
        ...binding,
        credentialStatus: stillConfigured
          ? binding.credentialStatus
          : "missing",
        providerCredentialStatus: stillConfigured ? "configured" : "missing",
      }));
      recomputeAgentBindingState();
      return success(owner.apiKey);
    },
  ),
  http.post(`${TAURI_ENDPOINT}/list_provider_api_keys`, async ({ request }) => {
    const { providerId } = await withJson<{ providerId: string }>(request);
    const provider = usageProvidersFixture.find(
      (candidate) => candidate.id === providerId,
    );
    return success(provider?.apiKeys ?? []);
  }),
  http.post(
    `${TAURI_ENDPOINT}/create_provider_api_key`,
    async ({ request }) => {
      const { providerId, label } = await withJson<{
        providerId: string;
        label: string;
      }>(request);
      const provider = usageProvidersFixture.find(
        (candidate) => candidate.id === providerId,
      );
      if (!provider) return rejectUsageRequest("unsupported_auth");
      if (provider.apiKeys.some((candidate) => candidate.label === label)) {
        return rejectUsageRequest("duplicate_label");
      }
      const created: ProviderApiKeyView = {
        id: `${providerId}-key-${provider.apiKeys.length + 1}`,
        providerId,
        label,
        credentialStatus: "missing",
        credentialVersion: 0,
        canClearCredential: false,
        lastConnectionTestAt: null,
        lastConnectionTestStatus: null,
        lastConnectionTestErrorCode: null,
        sortOrder: provider.apiKeys.length,
        keyUsage: null,
      };
      provider.apiKeys = [...provider.apiKeys, created];
      return success(created);
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/rename_provider_api_key`,
    async ({ request }) => {
      const { keyId, label } = await withJson<{ keyId: string; label: string }>(
        request,
      );
      const owner = findProviderApiKey(keyId);
      if (!owner) return rejectUsageRequest("credential_conflict");
      if (
        owner.provider.apiKeys.some(
          (candidate) => candidate.id !== keyId && candidate.label === label,
        )
      ) {
        return rejectUsageRequest("duplicate_label");
      }
      owner.apiKey.label = label;
      return success(owner.apiKey);
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/delete_provider_api_key`,
    async ({ request }) => {
      const { keyId, expectedVersion } = await withJson<{
        keyId: string;
        expectedVersion: number;
      }>(request);
      const owner = findProviderApiKey(keyId);
      if (!owner || owner.apiKey.credentialVersion !== expectedVersion) {
        return rejectUsageRequest("credential_conflict");
      }
      owner.provider.apiKeys = owner.provider.apiKeys.filter(
        (candidate) => candidate.id !== keyId,
      );
      return success(null);
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/test_system_provider_connection`,
    async ({ request }) => {
      const { keyId } = await withJson<{ keyId: string }>(request);
      const owner = findProviderApiKey(keyId);
      if (owner) {
        owner.apiKey.lastConnectionTestAt = 1_000;
        owner.apiKey.lastConnectionTestStatus = "success";
      }
      return success({
        providerId: owner?.provider.id ?? "",
        success: true,
        status: "success",
        testedAt: 1_000,
        errorCode: null,
      });
    },
  ),
  http.post(`${TAURI_ENDPOINT}/get_claude_cli_auth_status`, () =>
    success({
      installed: true,
      authenticated: false,
      subscriptionType: null,
      quotaAvailability: "unavailable",
      errorCode: null,
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/start_claude_cli_login`, () => success(null)),
  http.post(`${TAURI_ENDPOINT}/logout_claude_cli`, () =>
    success({
      installed: true,
      authenticated: false,
      subscriptionType: null,
      quotaAvailability: "unavailable",
      errorCode: null,
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/save_usage_provider`, async ({ request }) => {
    const { input } = await withJson<{
      input: UsageProviderInput & Record<string, unknown>;
    }>(request);
    if (input && Object.prototype.hasOwnProperty.call(input, "quotaConfig")) {
      return rejectUsageRequest("renderer_quota_config_forbidden");
    }
    if (
      input &&
      Object.prototype.hasOwnProperty.call(input, "dailyBudgetUsd")
    ) {
      return rejectUsageRequest("renderer_daily_budget_forbidden");
    }
    if (
      !input?.id?.trim() ||
      !input.name?.trim() ||
      !input.productGroupId?.trim()
    ) {
      return rejectUsageRequest("invalid_provider_input");
    }
    const existing = usageProvidersFixture.find(
      (provider) => provider.id === input.id,
    );
    const routeBaseUrl =
      typeof input.routeConfig?.baseUrl === "string"
        ? input.routeConfig.baseUrl
        : (existing?.routeBaseUrl ?? null);
    const saved: UsageProviderView = {
      ...customProviderSystemFields,
      id: input.id,
      name: input.name,
      billingKind: input.billingKind,
      productGroupId: input.productGroupId,
      tokenSources: [...input.tokenSources],
      sessionSourceBindings:
        input.sessionSourceBindings ?? existing?.sessionSourceBindings ?? [],
      bindings: existing?.bindings ?? [],
      quotaSource: input.quotaSource ?? null,
      quotaIntervalSeconds: input.quotaIntervalSeconds ?? null,
      dailyBudgetUsd: existing?.dailyBudgetUsd ?? null,
      routeAppType: input.routeAppType ?? null,
      enabled: input.enabled,
      needsReview: false,
      createdAt: existing?.createdAt ?? 1_000,
      updatedAt: (existing?.updatedAt ?? 999) + 1,
      routeBaseUrl,
      hasRouteCredentials:
        input.routeConfig == null
          ? (existing?.hasRouteCredentials ?? false)
          : typeof input.routeConfig.apiKey === "string",
    };
    if (existing) {
      usageProvidersFixture.splice(
        usageProvidersFixture.indexOf(existing),
        1,
        saved,
      );
    } else {
      usageProvidersFixture.push(saved);
    }
    recomputeAgentBindingState();
    return success(saved);
  }),
  http.post(
    `${TAURI_ENDPOINT}/set_usage_provider_enabled`,
    async ({ request }) => {
      const { providerId, enabled } = await withJson<{
        providerId: string;
        enabled: boolean;
      }>(request);
      const provider = usageProvidersFixture.find(
        (candidate) => candidate.id === providerId,
      );
      if (!provider || typeof enabled !== "boolean") {
        return rejectUsageRequest("invalid_provider_enabled");
      }
      provider.enabled = enabled;
      provider.updatedAt += 1;
      recomputeAgentBindingState();
      return success(null);
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/list_agent_provider_bindings`,
    async ({ request }) => {
      const { agentModuleId } = await withJson<{
        agentModuleId?: string;
      }>(request);
      return success(
        allAgentBindings().filter(
          (binding) =>
            agentModuleId == null || binding.agentModuleId === agentModuleId,
        ),
      );
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/save_agent_provider_binding`,
    async ({ request }) => {
      const { input } = await withJson<{ input: AgentProviderBindingInput }>(
        request,
      );
      if (
        !input ||
        !activeAgent(input.agentModuleId) ||
        !usageProvidersFixture.some(
          (provider) => provider.id === input.providerId,
        )
      ) {
        return rejectUsageRequest("invalid_agent_provider_binding");
      }
      const provider = usageProvider(input.providerId);
      if (input.id) {
        const location = bindingLocation(input.id);
        if (
          !location ||
          location.provider.id !== input.providerId ||
          location.provider.bindings[location.index].agentModuleId !==
            input.agentModuleId
        ) {
          return rejectUsageRequest("binding_identity_mismatch");
        }
        const current = location.provider.bindings[location.index];
        const saved: AgentProviderBindingView = {
          ...current,
          enabled: input.enabled,
          updatedAt: current.updatedAt + 1,
        };
        location.provider.bindings[location.index] = saved;
        recomputeAgentBindingState();
        return success(location.provider.bindings[location.index]);
      }
      if (
        provider.bindings.some(
          (binding) => binding.agentModuleId === input.agentModuleId,
        )
      ) {
        return rejectUsageRequest("duplicate_agent_provider_binding");
      }
      const timestamp = 1_000 + nextBindingId;
      const saved: AgentProviderBindingView = {
        ...customBindingCredentialFields,
        id: `binding-msw-${nextBindingId++}`,
        agentModuleId: input.agentModuleId,
        providerId: input.providerId,
        enabled: input.enabled,
        effectiveEnabled: false,
        credentialStatus:
          provider.billingKind === "subscription" ? "not_required" : "missing",
        canClearCredential: false,
        credentialVersion: 0,
        createdAt: timestamp,
        updatedAt: timestamp,
      };
      provider.bindings.push(saved);
      recomputeAgentBindingState();
      return success(
        provider.bindings.find((binding) => binding.id === saved.id)!,
      );
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/delete_agent_provider_binding`,
    async ({ request }) => {
      const { bindingId, expectedVersion } = await withJson<{
        bindingId: string;
        expectedVersion: number;
      }>(request);
      const location = bindingLocation(bindingId);
      if (
        !location ||
        location.provider.bindings[location.index].credentialVersion !==
          expectedVersion
      ) {
        return rejectUsageRequest("binding_version_mismatch");
      }
      location.provider.bindings.splice(location.index, 1);
      recomputeAgentBindingState();
      return success(null);
    },
  ),
  ...(
    [
      ["set_agent_provider_binding_api_key", "configured"],
      ["replace_agent_provider_binding_api_key", "configured"],
      ["clear_agent_provider_binding_api_key", "missing"],
    ] as const
  ).map(([command, credentialStatus]) =>
    http.post(`${TAURI_ENDPOINT}/${command}`, async ({ request }) => {
      const { bindingId, expectedVersion, apiKey } = await withJson<{
        bindingId: string;
        expectedVersion: number;
        apiKey?: string;
      }>(request);
      const location = bindingLocation(bindingId);
      if (
        !location ||
        location.provider.bindings[location.index].credentialVersion !==
          expectedVersion ||
        (credentialStatus === "configured" && !apiKey)
      ) {
        return rejectUsageRequest("binding_credential_update_rejected");
      }
      const current = location.provider.bindings[location.index];
      const saved: AgentProviderBindingView = {
        ...current,
        enabled: credentialStatus === "configured" ? current.enabled : false,
        credentialStatus,
        canClearCredential: credentialStatus === "configured",
        credentialVersion: current.credentialVersion + 1,
        updatedAt: current.updatedAt + 1,
      };
      location.provider.bindings[location.index] = saved;
      recomputeAgentBindingState();
      return success(location.provider.bindings[location.index]);
    }),
  ),
  http.post(
    `${TAURI_ENDPOINT}/get_agent_proxy_setup_info`,
    async ({ request }) => {
      const { agentModuleId } = await withJson<{ agentModuleId: string }>(
        request,
      );
      if (!activeAgent(agentModuleId)) {
        return rejectUsageRequest("agent_not_found");
      }
      const proxyOrigin = "http://127.0.0.1:15800";
      return success({
        agentModuleId,
        proxyRunning: false,
        proxyOrigin,
        routes: usageProvidersFixture.flatMap((provider) =>
          provider.bindings
            .filter((binding) => binding.agentModuleId === agentModuleId)
            .map((binding) => ({
              bindingId: binding.id,
              providerId: provider.id,
              protocol: provider.routeAppType,
              localBaseUrl: provider.routeAppType
                ? `${proxyOrigin}/${agentModuleId}/${provider.routeAppType}`
                : null,
              credentialPlacements:
                provider.routeAppType === "claude"
                  ? ["x-api-key"]
                  : provider.routeAppType === "codex"
                    ? ["Authorization"]
                    : provider.routeAppType === "gemini"
                      ? ["x-goog-api-key"]
                      : [],
              credentialStatus: binding.credentialStatus,
            })),
        ),
      });
    },
  ),
  http.post(`${TAURI_ENDPOINT}/get_unassigned_usage_diagnostics`, () =>
    success({
      unassignedEventCount: 4,
      unassignedGroups: [
        {
          providerId: "legacy-provider",
          source: "proxy",
          eventCount: 4,
          firstOccurredAt: 10,
          lastOccurredAt: 20,
        },
      ],
      archivedAgentHistory: [
        {
          agentModuleId: "custom-archived",
          eventCount: 3,
          firstOccurredAt: 30,
          lastOccurredAt: 40,
        },
      ],
      invalidLinkSummaries: [
        {
          reason: "cross_agent",
          linkCount: 2,
          firstCreatedAt: 50,
          lastCreatedAt: 60,
        },
      ],
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/refresh_provider_quota`, async ({ request }) => {
    const { providerId } = await withJson<{ providerId: string }>(request);
    usageProvider(providerId);
    return success({
      snapshot: {
        snapshotId: `quota-refreshed-${providerId}`,
        fetchedAt: 1_000,
        fiveHourUtilizationPercent: "26",
        fiveHourResetsAt: null,
        sevenDayUtilizationPercent: "41",
        sevenDayResetsAt: null,
        manualResetsRemaining: 1,
      },
      fetchState: {
        providerId,
        lastAttemptAt: 1_000,
        lastSuccessAt: 1_000,
        lastError: null,
        stale: false,
      },
    });
  }),
  http.post(
    `${TAURI_ENDPOINT}/sync_provider_session_usage`,
    async ({ request }) => {
      const { providerId } = await withJson<{ providerId: string }>(request);
      usageProvider(providerId);
      return success({
        imported: 1,
        skipped: 0,
        filesScanned: 1,
        errors: [],
        warnings: [],
      });
    },
  ),
  http.post(`${TAURI_ENDPOINT}/get_usage_dashboard`, async ({ request }) => {
    const { agentModuleId, startAt, endAt } = await withJson<{
      agentModuleId: string;
      startAt: number;
      endAt: number;
    }>(request);
    if (
      !activeAgent(agentModuleId) ||
      typeof startAt !== "number" ||
      typeof endAt !== "number"
    ) {
      return HttpResponse.json("invalid_agent_dashboard_request", {
        status: 400,
      });
    }
    return success({
      agentModuleId,
      startAt,
      endAt,
      warnings: [],
      productGroups: dashboardGroupsForAgent(agentModuleId, endAt),
    });
  }),
  http.post(
    `${TAURI_ENDPOINT}/get_provider_usage_dashboard`,
    async ({ request }) => {
      const { startAt, endAt } = await withJson<{
        startAt: number;
        endAt: number;
      }>(request);
      if (typeof startAt !== "number" || typeof endAt !== "number") {
        return HttpResponse.json("invalid_provider_dashboard_request", {
          status: 400,
        });
      }
      return success({
        startAt,
        endAt,
        warnings: [],
        trendGranularity: endAt - startAt <= 24 * 60 * 60 ? "hour" : "day",
        trendBuckets: [
          {
            startAt,
            endAt,
            eventCount: 3,
            inputTokens: 500,
            outputTokens: 250,
            cacheReadTokens: 500,
            cacheCreationTokens: 0,
            totalTokens: 1_250,
            totalCostUsd: "0.125",
            costSourceCounts: {
              upstream: 1,
              estimated: 2,
              unavailable: 0,
            },
          },
        ],
        providers: usageProvidersFixture
          .filter((provider) => provider.enabled)
          .map((provider, index) =>
            provider.billingKind === "subscription"
              ? subscriptionUsage(provider.id, endAt, 100 + index)
              : meteredUsage(
                  provider.id,
                  200 + index,
                  provider.id === "anthropic-api" ? null : "1.25",
                ),
          ),
      });
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/get_model_usage_dashboard`,
    async ({ request }) => {
      const { startAt, endAt } = await withJson<{
        startAt: number;
        endAt: number;
      }>(request);
      if (typeof startAt !== "number" || typeof endAt !== "number") {
        return HttpResponse.json("invalid_model_dashboard_request", {
          status: 400,
        });
      }
      return success(modelUsageDashboardFixture(startAt, endAt));
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/get_agent_usage_breakdown`,
    async ({ request }) => {
      const { startAt, endAt } = await withJson<{
        startAt: number;
        endAt: number;
      }>(request);
      if (typeof startAt !== "number" || typeof endAt !== "number") {
        return HttpResponse.json("invalid_agent_breakdown_request", {
          status: 400,
        });
      }
      return success(agentUsageBreakdownFixture(startAt, endAt));
    },
  ),
  http.post(
    `${TAURI_ENDPOINT}/get_provider_usage_activity`,
    async ({ request }) => {
      const { startAt, endAt } = await withJson<{
        startAt: number;
        endAt: number;
      }>(request);
      if (typeof startAt !== "number" || typeof endAt !== "number") {
        return HttpResponse.json("invalid_provider_activity_request", {
          status: 400,
        });
      }
      return success([
        {
          startAt,
          endAt: Math.min(startAt + 24 * 60 * 60, endAt),
          eventCount: 3,
          inputTokens: 500,
          outputTokens: 250,
          cacheReadTokens: 500,
          cacheCreationTokens: 0,
          totalTokens: 1_250,
          totalCostUsd: "0.125",
          costSourceCounts: {
            upstream: 1,
            estimated: 2,
            unavailable: 0,
          },
        },
      ]);
    },
  ),
  http.post(`${TAURI_ENDPOINT}/get_usage_events`, async ({ request }) => {
    const { agentModuleId, providerId } = await withJson<{
      agentModuleId: string;
      providerId?: string;
    }>(request);
    if (
      !activeAgent(agentModuleId) ||
      (providerId &&
        !usageProvidersFixture.some((provider) => provider.id === providerId))
    ) {
      return HttpResponse.json("invalid_agent_event_request", { status: 400 });
    }
    const event =
      providerId === "azure-api"
        ? {
            eventId: `event-${agentModuleId}-azure`,
            source: "proxy",
            providerId,
            agentModuleId,
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
              eventId: `event-${agentModuleId}-openrouter`,
              source: "proxy",
              providerId,
              agentModuleId,
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
  http.post(
    `${TAURI_ENDPOINT}/get_provider_usage_events`,
    async ({ request }) => {
      const { providerId } = await withJson<{ providerId: string }>(request);
      usageProvider(providerId);
      return success({
        items: [
          {
            eventId: `event-${providerId}`,
            source: "proxy",
            providerId,
            agentModuleId: null,
            productGroupId: usageProvider(providerId).productGroupId,
            occurredAt: 5,
            model: "provider-model",
            inputTokens: 50,
            outputTokens: 10,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            requestId: "request-provider",
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
          },
        ],
        total: 1,
        page: 1,
        pageSize: 5,
      });
    },
  ),
  http.post(`${TAURI_ENDPOINT}/is_proxy_running`, () => success(false)),
  http.post(`${TAURI_ENDPOINT}/get_migration_result`, () => success(false)),
  http.post(`${TAURI_ENDPOINT}/get_skills_migration_result`, () =>
    success(null),
  ),
  http.post(`${TAURI_ENDPOINT}/update_tray_menu`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_openclaw_default_model`, () =>
    success({ primary: null, fallback: [] }),
  ),

  http.post(`${TAURI_ENDPOINT}/scan_openclaw_config_health`, () => success([])),

  http.post(`${TAURI_ENDPOINT}/open_external`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/import_mcp_from_claude`, () => success(1)),
  http.post(`${TAURI_ENDPOINT}/import_mcp_from_codex`, () => success(1)),

  http.post(`${TAURI_ENDPOINT}/restart_app`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/check_env_conflicts`, () => success([])),

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
