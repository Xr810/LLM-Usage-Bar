import type { ManualResetCreditView } from "./quota";

export type TrayUsageStatus = "green" | "yellow" | "red" | "unknown";

export type PaceBasis = "measured" | "window_average" | "static" | "idle";

export type TrayCostQuality =
  "complete" | "estimated" | "partial" | "unavailable";

/** BE-21b. Absent until the snapshot publishes a build. */
export type TrayBuildType = "community" | "official";

export interface TrayUsageSnapshot {
  status: TrayUsageStatus;
  generatedAt: number;
  lastSuccessAt: number | null;
  stale: boolean;
  refreshError: string | null;
  refreshInProgress: boolean;
  apiBudget: TrayApiBudgetView;
  agents: TrayAgentUsageView[];
  /** BE-21b. Null or absent when the snapshot has no build yet. */
  buildType?: TrayBuildType | null;
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
  /**
   * C10 budget level. Pace `status` is display-only and must not set warn.
   * Absent until BE-20; the panel then keeps the old status fallback.
   */
  level?: "ok" | "near" | "over";
  warningReason: string | null;
  burnRateUsdPerHour?: string | null;
  projectedExhaustAt?: string | null;
  headroomRatio?: string | null;
  paceBasis?: PaceBasis;
  rhythmAdjustment?: string | null;
  flatStatus?: TrayUsageStatus | null;
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
  /**
   * BE-21b. Codes such as `codex_websocket`. Null or absent means no note.
   */
  noteCodes?: string[] | null;
  /**
   * BE-21b. Next reset instant. Null or absent means unknown — do not
   * substitute local midnight.
   */
  nextResetAt?: string | null;
}

export interface TrayProviderRecentUsageView {
  startAt: number;
  endAt: number;
  /** Tokens for the local day — same window as `todayCostUsd`. */
  todayTokens: number;
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
  burnRatePercentPerHour?: string | null;
  projectedExhaustAt?: string | null;
  headroomRatio?: string | null;
  paceBasis?: PaceBasis;
  rhythmAdjustment?: string | null;
  flatStatus?: TrayUsageStatus | null;
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
  rhythmAdjustment?: string | null;
  flatStatus?: TrayUsageStatus | null;
}

export type MainWindowDestination =
  | { kind: "usage"; agentModuleId: string | null }
  | { kind: "providerBudget"; providerId: string | null };
