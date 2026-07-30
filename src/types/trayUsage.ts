import type { ManualResetCreditView } from "./quota";

export type TrayUsageStatus = "green" | "yellow" | "red" | "unknown";

export type TrayCostQuality =
  "complete" | "estimated" | "partial" | "unavailable";

export interface TrayUsageSnapshot {
  status: TrayUsageStatus;
  generatedAt: number;
  lastSuccessAt: number | null;
  stale: boolean;
  refreshError: string | null;
  refreshInProgress: boolean;
  apiBudget: TrayApiBudgetView;
  agents: TrayAgentUsageView[];
}

export type ApiBudgetMode = "shared" | "per_provider";

export interface ApiBudgetConfig {
  mode: ApiBudgetMode;
  sharedDailyBudgetUsd: string | null;
}

export interface TrayApiBudgetView {
  mode: ApiBudgetMode;
  providerCount: number;
  todayCostUsd: string | null;
  dailyBudgetUsd: string | null;
  budgetConsumedPercent: string | null;
  costQuality: TrayCostQuality;
  status: TrayUsageStatus;
  warningReason: string | null;
}

export interface TrayAgentUsageView {
  agentModuleId: string;
  name: string;
  sortOrder: number;
  status: TrayUsageStatus;
  providers: TrayProviderUsageView[];
}

export interface TrayProviderUsageView {
  providerId: string;
  providerName: string;
  systemPresetKey: string | null;
  billingKind: "subscription" | "metered";
  status: TrayUsageStatus;
  warningReason: string | null;
  recentUsage: TrayProviderRecentUsageView;
  subscription: TraySubscriptionUsageView | null;
  metered: TrayMeteredUsageView | null;
}

export interface TrayProviderRecentUsageView {
  startAt: number;
  endAt: number;
  totalTokens: number;
  todayCostUsd: string | null;
  totalCostUsd: string | null;
  costQuality: TrayCostQuality;
  mostUsedModel: string | null;
  trendBuckets: TrayUsageTrendBucketView[];
}

export interface TrayUsageTrendBucketView {
  startAt: number;
  endAt: number;
  eventCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
  totalCostUsd: string | null;
  costSourceCounts: {
    upstream: number;
    estimated: number;
    unavailable: number;
  };
}

export interface TrayQuotaWindowView {
  kind: "five_hour" | "seven_day";
  usedPercent: string | null;
  remainingPercent: string | null;
  resetsAt: string | null;
  status: TrayUsageStatus;
  unavailableReason: string | null;
}

export interface TraySubscriptionUsageView {
  planLabel: string | null;
  windows: TrayQuotaWindowView[];
  manualResetsRemaining?: number | null;
  manualResetCredits?: ManualResetCreditView[];
}

export interface TrayMeteredUsageView {
  todayCostUsd: string | null;
  rolling30DayCostUsd: string | null;
  dailyBudgetUsd: string | null;
  budgetConsumedPercent: string | null;
  totalTokens: number;
  costQuality: TrayCostQuality;
}

export type MainWindowDestination =
  | { kind: "usage"; agentModuleId: string | null }
  | { kind: "providerBudget"; providerId: string | null };
