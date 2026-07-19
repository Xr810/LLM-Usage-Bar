import type {
  AgentModuleView,
  CostSourceCounts,
  ProviderUsageView,
  QuotaFetchState,
  QuotaStatusView,
  UsageDashboardView,
  ProviderMonitoringDashboardView,
  UsageTrendBucketView,
  UsageTrendGranularity,
} from "@/types/usageDashboard";

export type MeteredCostStatus =
  "complete" | "estimated" | "partial" | "unavailable";

export interface AgentUsageProjection {
  agent: AgentModuleView;
  subscriptionProviders: ProviderUsageView[];
  meteredProviders: ProviderUsageView[];
  meteredTotalTokens: number;
  meteredRequestCount: number;
  meteredTotalCostUsd: string | null;
  meteredCostStatus: MeteredCostStatus;
}

export interface ProviderDashboardProjection {
  subscriptionProviders: ProviderUsageView[];
  meteredProviders: ProviderUsageView[];
  overallTotalTokens: number;
  overallRequestCount: number;
  overallTotalCostUsd: string | null;
  overallCostStatus: MeteredCostStatus;
  meteredTotalTokens: number;
  meteredRequestCount: number;
  meteredTotalCostUsd: string | null;
  meteredCostStatus: MeteredCostStatus;
  trendGranularity: UsageTrendGranularity;
  trendBuckets: UsageTrendBucketView[];
}

function addCounts(
  left: CostSourceCounts,
  right: CostSourceCounts,
): CostSourceCounts {
  return {
    upstream: left.upstream + right.upstream,
    estimated: left.estimated + right.estimated,
    unavailable: left.unavailable + right.unavailable,
  };
}

function newestQuota(
  left: QuotaStatusView | null,
  right: QuotaStatusView | null,
): QuotaStatusView | null {
  if (!left) return right;
  if (!right) return left;
  return right.fetchedAt >= left.fetchedAt ? right : left;
}

function newestFetchState(
  left: QuotaFetchState | null,
  right: QuotaFetchState | null,
): QuotaFetchState | null {
  if (!left) return right;
  if (!right) return left;
  return (right.lastAttemptAt ?? -1) >= (left.lastAttemptAt ?? -1)
    ? right
    : left;
}

export function addDecimalStrings(values: string[]): string | null {
  if (!values.length) return null;
  const parsed = values.map((value) => {
    const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value);
    if (!match) throw new Error(`Invalid decimal value: ${value}`);
    return {
      negative: match[1] === "-",
      whole: match[2],
      fraction: match[3] ?? "",
    };
  });
  const scale = Math.max(...parsed.map((value) => value.fraction.length));
  const total = parsed.reduce((sum, value) => {
    const units = BigInt(`${value.whole}${value.fraction.padEnd(scale, "0")}`);
    return sum + (value.negative ? -units : units);
  }, 0n);
  const negative = total < 0n;
  const digits = (negative ? -total : total)
    .toString()
    .padStart(scale + 1, "0");
  if (!scale) return `${negative ? "-" : ""}${digits}`;
  const whole = digits.slice(0, -scale);
  const fraction = digits.slice(-scale).replace(/0+$/, "");
  return `${negative ? "-" : ""}${whole}${fraction ? `.${fraction}` : ""}`;
}

function mergeProviderRows(rows: ProviderUsageView[]): ProviderUsageView[] {
  const merged = new Map<string, ProviderUsageView>();
  for (const row of rows) {
    const current = merged.get(row.provider.id);
    if (!current) {
      merged.set(row.provider.id, {
        ...row,
        costSourceCounts: { ...row.costSourceCounts },
      });
      continue;
    }

    const provider =
      row.provider.updatedAt >= current.provider.updatedAt
        ? row.provider
        : current.provider;
    merged.set(row.provider.id, {
      provider,
      sharedAccount: current.sharedAccount || row.sharedAccount,
      eventCount: current.eventCount + row.eventCount,
      inputTokens: current.inputTokens + row.inputTokens,
      outputTokens: current.outputTokens + row.outputTokens,
      cacheReadTokens: current.cacheReadTokens + row.cacheReadTokens,
      cacheCreationTokens:
        current.cacheCreationTokens + row.cacheCreationTokens,
      totalCostUsd: addDecimalStrings(
        [current.totalCostUsd, row.totalCostUsd].filter(
          (value): value is string => value != null,
        ),
      ),
      costSourceCounts: addCounts(
        current.costSourceCounts,
        row.costSourceCounts,
      ),
      quota: newestQuota(current.quota, row.quota),
      quotaFetchState: newestFetchState(
        current.quotaFetchState,
        row.quotaFetchState,
      ),
    });
  }
  return [...merged.values()];
}

export function projectAgentDashboard(
  agent: AgentModuleView,
  dashboard: UsageDashboardView,
): AgentUsageProjection | null {
  if (dashboard.agentModuleId !== agent.id) return null;

  const providers = mergeProviderRows(
    dashboard.productGroups.flatMap((group) => [
      ...group.subscriptionProviders,
      ...group.meteredProviders,
    ]),
  );
  const subscriptionProviders = providers.filter(
    (usage) => usage.provider.billingKind === "subscription",
  );
  const meteredProviders = providers.filter(
    (usage) => usage.provider.billingKind === "metered",
  );
  const knownCosts = meteredProviders
    .map((usage) => usage.totalCostUsd)
    .filter((value): value is string => value != null);
  const meteredTotalCostUsd = addDecimalStrings(knownCosts);
  const hasUnavailable = meteredProviders.some(
    (usage) =>
      usage.totalCostUsd == null || usage.costSourceCounts.unavailable > 0,
  );
  const hasEstimated = meteredProviders.some(
    (usage) => usage.costSourceCounts.estimated > 0,
  );
  const meteredCostStatus: MeteredCostStatus =
    meteredTotalCostUsd == null
      ? "unavailable"
      : hasUnavailable
        ? "partial"
        : hasEstimated
          ? "estimated"
          : "complete";

  return {
    agent,
    subscriptionProviders,
    meteredProviders,
    meteredTotalTokens: meteredProviders.reduce(
      (sum, usage) =>
        sum +
        usage.inputTokens +
        usage.outputTokens +
        usage.cacheReadTokens +
        usage.cacheCreationTokens,
      0,
    ),
    meteredRequestCount: meteredProviders.reduce(
      (sum, usage) => sum + usage.eventCount,
      0,
    ),
    meteredTotalCostUsd,
    meteredCostStatus,
  };
}

export function projectProviderDashboard(
  dashboard: ProviderMonitoringDashboardView,
): ProviderDashboardProjection {
  const providers = mergeProviderRows(dashboard.providers);
  const subscriptionProviders = providers.filter(
    (usage) => usage.provider.billingKind === "subscription",
  );
  const meteredProviders = providers.filter(
    (usage) => usage.provider.billingKind === "metered",
  );
  const overallKnownCosts = providers
    .map((usage) => usage.totalCostUsd)
    .filter((value): value is string => value != null);
  const overallHasEvents = providers.some((usage) => usage.eventCount > 0);
  const overallTotalCostUsd = overallHasEvents
    ? addDecimalStrings(overallKnownCosts)
    : "0";
  const overallHasUnavailable = providers.some(
    (usage) =>
      usage.eventCount > 0 &&
      (usage.totalCostUsd == null || usage.costSourceCounts.unavailable > 0),
  );
  const overallHasEstimated = providers.some(
    (usage) => usage.costSourceCounts.estimated > 0,
  );
  const overallCostStatus: MeteredCostStatus =
    overallTotalCostUsd == null
      ? "unavailable"
      : overallHasUnavailable
        ? "partial"
        : overallHasEstimated
          ? "estimated"
          : "complete";
  const knownCosts = meteredProviders
    .map((usage) => usage.totalCostUsd)
    .filter((value): value is string => value != null);
  const meteredTotalCostUsd = addDecimalStrings(knownCosts);
  const hasUnavailable = meteredProviders.some(
    (usage) =>
      usage.totalCostUsd == null || usage.costSourceCounts.unavailable > 0,
  );
  const hasEstimated = meteredProviders.some(
    (usage) => usage.costSourceCounts.estimated > 0,
  );
  const meteredCostStatus: MeteredCostStatus =
    meteredTotalCostUsd == null
      ? "unavailable"
      : hasUnavailable
        ? "partial"
        : hasEstimated
          ? "estimated"
          : "complete";

  return {
    subscriptionProviders,
    meteredProviders,
    overallTotalTokens: providers.reduce(
      (sum, usage) =>
        sum +
        usage.inputTokens +
        usage.outputTokens +
        usage.cacheReadTokens +
        usage.cacheCreationTokens,
      0,
    ),
    overallRequestCount: providers.reduce(
      (sum, usage) => sum + usage.eventCount,
      0,
    ),
    overallTotalCostUsd,
    overallCostStatus,
    meteredTotalTokens: meteredProviders.reduce(
      (sum, usage) =>
        sum +
        usage.inputTokens +
        usage.outputTokens +
        usage.cacheReadTokens +
        usage.cacheCreationTokens,
      0,
    ),
    meteredRequestCount: meteredProviders.reduce(
      (sum, usage) => sum + usage.eventCount,
      0,
    ),
    meteredTotalCostUsd,
    meteredCostStatus,
    trendGranularity: dashboard.trendGranularity,
    trendBuckets: dashboard.trendBuckets,
  };
}
