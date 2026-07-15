export type BillingKind = "subscription" | "metered";
export type TokenSource = "proxy" | "session_log";
export type SessionSource = "claude" | "codex";
export type CostSource = "upstream" | "estimated" | "unavailable";
export type BindingCredentialStatus =
  "not_required" | "missing" | "configured" | "unavailable";
export type SystemProviderAuthKind =
  | "codex_oauth"
  | "claude_cli"
  | "provider_api_key";

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
  subscriptionType: "pro" | "max" | null;
  quotaAvailability: "unavailable";
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
  quotaConfig?: Record<string, unknown> | null;
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
  fiveHourUtilizationPercent: string | null;
  fiveHourResetsAt: string | null;
  sevenDayUtilizationPercent: string | null;
  sevenDayResetsAt: string | null;
  manualResetsRemaining: number | null;
}

export interface QuotaFetchState {
  providerId: string;
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  lastError: string | null;
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
