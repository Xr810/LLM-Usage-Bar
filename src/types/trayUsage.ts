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
  agents: TrayAgentUsageView[];
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
  subscription: TraySubscriptionUsageView | null;
  metered: TrayMeteredUsageView | null;
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
