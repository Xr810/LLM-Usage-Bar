import type { ManualResetCreditView } from "./quota";

export type BillingKind = "subscription" | "metered";
export type TokenSource = "proxy" | "session_log";
export type SessionSource = "claude" | "codex";
export type CostSource = "upstream" | "estimated" | "unavailable";

/**
 * `user` means the account's own price — what you actually pay. `official`
 * means the built-in reference catalogue, which is also what subscription
 * equivalent-API costs are always valued at.
 */
export type PricingOrigin = "user" | "official";

/** The four per-million-token rates that make up one model's price. */
/** What the user typed. A blank rate means "use the official one". */
export interface ModelPriceInput {
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

/**
 * One Provider account's own price for one model, in USD per million tokens.
 * A null rate was left blank and resolves to the official catalogue value.
 */
export interface ProviderModelPricingView {
  providerId: string;
  modelId: string;
  displayName: string;
  inputCostPerMillion: string | null;
  outputCostPerMillion: string | null;
  cacheReadCostPerMillion: string | null;
  cacheCreationCostPerMillion: string | null;
  updatedAt: number;
}
export type BindingCredentialStatus =
  "not_required" | "missing" | "configured" | "unavailable";
export type SystemProviderAuthKind =
  "codex_oauth" | "claude_cli" | "provider_api_key";

export interface AgentModuleView {
  id: string;
  name: string;
  sortOrder: number;
  visible: boolean;
  isFixed: boolean;
  archivedAt: number | null;
  providerCount: number;
}

export interface AgentModuleInput {
  id: string | null;
  name: string;
  sortOrder: number;
  visible: boolean;
}

export interface AgentProviderBindingInput {
  id: string | null;
  agentModuleId: string;
  providerId: string;
  enabled: boolean;
}

export interface AgentProviderBindingView {
  id: string;
  agentModuleId: string;
  providerId: string;
  enabled: boolean;
  effectiveEnabled: boolean;
  credentialStatus: BindingCredentialStatus;
  canClearCredential: boolean;
  credentialVersion: number;
  routeProtocol: string | null;
  localCredentialStatus: BindingCredentialStatus;
  providerCredentialStatus: BindingCredentialStatus;
  createdAt: number;
  updatedAt: number;
}

export interface AgentProxyRouteSetup {
  bindingId: string;
  providerId: string;
  protocol: string | null;
  localBaseUrl: string | null;
  credentialPlacements: string[];
  credentialStatus: BindingCredentialStatus;
}

export interface AgentProxySetupInfo {
  agentModuleId: string;
  proxyRunning: boolean;
  proxyOrigin: string;
  routes: AgentProxyRouteSetup[];
}

export interface LocalBindingKeyReveal {
  bindingId: string;
  credentialVersion: number;
  localKey: string;
}

export interface SystemProviderConnectionTestResult {
  providerId: string;
  success: boolean;
  status: "success" | "failed";
  testedAt: number;
  errorCode: string | null;
}

export interface ClaudeCliAuthStatus {
  installed: boolean;
  authenticated: boolean;
  authMethod: "api_key" | "claude_account" | "other" | null;
  subscriptionType: "pro" | "max" | null;
  /** When the newest local quota sample was taken; quota never comes from the CLI. */
  lastQuotaSampleAt: number | null;
  errorCode: string | null;
}

export interface UsageProviderInput {
  id: string;
  name: string;
  billingKind: BillingKind;
  productGroupId: string;
  tokenSources: TokenSource[];
  sessionSourceBindings?: SessionSource[] | null;
  quotaSource?: string | null;
  quotaIntervalSeconds?: number | null;
  routeAppType?: string | null;
  routeConfig?: Record<string, unknown> | null;
  enabled: boolean;
}

export interface UsageProviderView {
  id: string;
  name: string;
  billingKind: BillingKind;
  productGroupId: string;
  tokenSources: TokenSource[];
  sessionSourceBindings: SessionSource[];
  bindings: AgentProviderBindingView[];
  quotaSource: string | null;
  quotaIntervalSeconds: number | null;
  dailyBudgetUsd: string | null;
  routeAppType: string | null;
  enabled: boolean;
  needsReview: boolean;
  createdAt: number;
  updatedAt: number;
  routeBaseUrl: string | null;
  hasRouteCredentials: boolean;
  systemPresetKey: string | null;
  systemAuthKind: SystemProviderAuthKind | null;
  canonicalEndpoint: string | null;
  compatibleAgentModuleIds: string[];
  upstreamCredentialStatus: BindingCredentialStatus;
  upstreamCredentialVersion: number;
  canClearUpstreamCredential: boolean;
  lastConnectionTestAt: number | null;
  lastConnectionTestStatus: string | null;
}

export interface CostSourceCounts {
  upstream: number;
  estimated: number;
  unavailable: number;
}

export interface QuotaStatusView {
  snapshotId: string;
  fetchedAt: number;
  sourceObservedAt?: number | null;
  fiveHourUtilizationPercent: string | null;
  fiveHourResetsAt: string | null;
  sevenDayUtilizationPercent: string | null;
  sevenDayResetsAt: string | null;
  manualResetsRemaining: number | null;
  manualResetCredits?: ManualResetCreditView[];
}

export interface QuotaFetchState {
  providerId: string;
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  lastError: string | null;
  consecutiveFailures: number;
  stale: boolean;
}

export interface ProviderUsageView {
  provider: UsageProviderView;
  sharedAccount: boolean;
  eventCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalCostUsd: string | null;
  costSourceCounts: CostSourceCounts;
  quota: QuotaStatusView | null;
  quotaFetchState: QuotaFetchState | null;
}

export type UsageTrendGranularity = "hour" | "day";

export interface UsageTrendBucketView {
  startAt: number;
  endAt: number;
  eventCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
  totalCostUsd: string | null;
  costSourceCounts: CostSourceCounts;
}

export interface ProductUsageView {
  productGroupId: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalCostUsd: string | null;
  costSourceCounts: CostSourceCounts;
  tokenSources: TokenSource[];
  subscriptionProviders: ProviderUsageView[];
  meteredProviders: ProviderUsageView[];
}

export interface UsageDashboardView {
  agentModuleId: string;
  startAt: number;
  endAt: number;
  productGroups: ProductUsageView[];
  warnings: string[];
}

export interface ProviderMonitoringDashboardView {
  startAt: number;
  endAt: number;
  providers: ProviderUsageView[];
  trendGranularity: UsageTrendGranularity;
  trendBuckets: UsageTrendBucketView[];
  warnings: string[];
}

export interface UsageEvent {
  eventId: string;
  source: TokenSource;
  providerId: string;
  agentModuleId: string | null;
  productGroupId: string;
  occurredAt: number;
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  requestId: string | null;
  sessionId: string | null;
  upstreamCorrelationId: string | null;
  inputCostUsd: string | null;
  outputCostUsd: string | null;
  cacheReadCostUsd: string | null;
  cacheCreationCostUsd: string | null;
  totalCostUsd: string | null;
  costSource: CostSource;
  /**
   * Which price catalogue produced an estimate. Null for upstream-reported and
   * unavailable costs, and for events recorded before schema v20.
   */
  pricingOrigin: PricingOrigin | null;
  legacyRequestId: string | null;
  createdAt: number;
}

export interface UsageEventPage {
  items: UsageEvent[];
  total: number;
  page: number;
  pageSize: number;
}

export interface UnassignedUsageGroup {
  providerId: string;
  source: TokenSource;
  eventCount: number;
  firstOccurredAt: number;
  lastOccurredAt: number;
}

export interface ArchivedAgentUsageSummary {
  agentModuleId: string;
  eventCount: number;
  firstOccurredAt: number;
  lastOccurredAt: number;
}

export interface InvalidUsageLinkSummary {
  reason: string;
  linkCount: number;
  firstCreatedAt: number;
  lastCreatedAt: number;
}

export interface UnassignedUsageDiagnostics {
  unassignedEventCount: number;
  unassignedGroups: UnassignedUsageGroup[];
  archivedAgentHistory: ArchivedAgentUsageSummary[];
  invalidLinkSummaries: InvalidUsageLinkSummary[];
}

export interface QuotaRefreshResult {
  snapshot: QuotaStatusView;
  fetchState: QuotaFetchState;
}

export interface ProviderSessionSyncResult {
  imported: number;
  skipped: number;
  filesScanned: number;
  errors: string[];
  warnings: string[];
}
