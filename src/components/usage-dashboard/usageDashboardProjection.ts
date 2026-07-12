import type {
  CostSourceCounts,
  DashboardModuleView,
  ProviderUsageView,
  QuotaFetchState,
  QuotaStatusView,
  UsageDashboardView,
} from "@/types/usageDashboard";

export type ApiCostStatus =
  "complete" | "estimated" | "partial" | "unavailable";

export interface SubscriptionModuleProjection {
  kind: "subscription";
  module: DashboardModuleView;
  providers: ProviderUsageView[];
}

export interface ApiModuleProjection {
  kind: "api";
  module: DashboardModuleView;
  providers: ProviderUsageView[];
  totalTokens: number;
  requestCount: number;
  totalCostUsd: string | null;
  costStatus: ApiCostStatus;
}

export type DashboardModuleProjection =
  SubscriptionModuleProjection | ApiModuleProjection;

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

export function projectDashboardModule(
  module: DashboardModuleView,
  dashboard: UsageDashboardView,
): DashboardModuleProjection {
  const rows = dashboard.productGroups.flatMap((group) => [
    ...group.subscriptionProviders,
    ...group.meteredProviders,
  ]);
  const merged = mergeProviderRows(rows);

  if (module.kind === "subscription") {
    return {
      kind: "subscription",
      module,
      providers: merged.filter(
        (usage) =>
          usage.provider.enabled &&
          usage.provider.billingKind === "subscription" &&
          usage.provider.dashboardModuleId === module.id,
      ),
    };
  }

  const providers = merged.filter(
    (usage) =>
      usage.provider.enabled && usage.provider.billingKind === "metered",
  );
  const knownCosts = providers
    .map((usage) => usage.totalCostUsd)
    .filter((value): value is string => value != null);
  const totalCostUsd = addDecimalStrings(knownCosts);
  const hasUnavailable = providers.some(
    (usage) =>
      usage.totalCostUsd == null || usage.costSourceCounts.unavailable > 0,
  );
  const hasEstimated = providers.some(
    (usage) => usage.costSourceCounts.estimated > 0,
  );
  const costStatus: ApiCostStatus =
    totalCostUsd == null
      ? "unavailable"
      : hasUnavailable
        ? "partial"
        : hasEstimated
          ? "estimated"
          : "complete";

  return {
    kind: "api",
    module,
    providers,
    totalTokens: providers.reduce(
      (sum, usage) =>
        sum +
        usage.inputTokens +
        usage.outputTokens +
        usage.cacheReadTokens +
        usage.cacheCreationTokens,
      0,
    ),
    requestCount: providers.reduce((sum, usage) => sum + usage.eventCount, 0),
    totalCostUsd,
    costStatus,
  };
}
