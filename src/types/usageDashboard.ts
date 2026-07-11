export type BillingKind = "subscription" | "metered";
export type TokenSource = "proxy" | "session_log";
export type CostSource = "upstream" | "estimated" | "unavailable";

export interface UsageProviderInput {
  id: string;
  name: string;
  billingKind: BillingKind;
  productGroupId: string;
  tokenSources: TokenSource[];
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
  quotaSource: string | null;
  quotaIntervalSeconds: number | null;
  routeAppType: string | null;
  enabled: boolean;
  needsReview: boolean;
  createdAt: number;
  updatedAt: number;
  routeBaseUrl: string | null;
  hasRouteCredentials: boolean;
}

export interface RouteBinding {
  protocol: string;
  providerId: string;
  updatedAt: number;
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
  startAt: number;
  endAt: number;
  productGroups: ProductUsageView[];
  warnings: string[];
}

export interface UsageEvent {
  eventId: string;
  source: TokenSource;
  providerId: string;
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
