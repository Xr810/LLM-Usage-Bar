import { useMemo, useState } from "react";
import { useProviderUsageActivity } from "@/lib/query/providerUsageActivity";
import { useApiBudgetConfig } from "@/lib/query/trayUsage";
import {
  useAgentUsageBreakdown,
  useProviderUsageDashboard,
} from "@/lib/query/usageDashboard";
import {
  clientSeries,
  dayPoints,
  heatmapCells,
  heatmapWindow,
  last30DaysRange,
  projectSeries,
  providerSeries,
  referenceLines,
  rollupPoints,
  sharedCapUsd,
  summarize,
  WEEK_STARTS_ON_MONDAY,
  type CostPoint,
  type HeatCell,
  type TrendDimension,
  type TrendGranularity,
  type TrendReferenceLine,
  type TrendSeries,
} from "./trendsModel";

export interface TrendsData {
  /** C19. Always the requested window, not the tier. */
  maxWeeks: number;
  /** C19. True only when the payload is wider than `maxWeeks`. */
  truncated: boolean;
  weekStartsOn: number;
  points: CostPoint[];
  series: TrendSeries[];
  lines: TrendReferenceLine[];
  totalUsd: number | null;
  averageUsd: number | null;
  peakUsd: number | null;
  capUsd: number | null;
  heatmap: HeatCell[];
  ready: boolean;
}

/**
 * Last 30 days from `get_provider_usage_dashboard` (day buckets: the window is
 * longer than 24h) and a 52-week heatmap from `get_provider_usage_activity`.
 *
 * Week and month are rolled up on the client. Project and client series use
 * each group's share of the range total: the dashboard trend is one series of
 * daily totals, and this batch does not add a per-group trend command.
 */
export function useTrendsData(
  dimension: TrendDimension,
  granularity: TrendGranularity,
  unattributedLabel: string,
): TrendsData {
  const [nowMs] = useState(() => Date.now());
  const range = useMemo(() => last30DaysRange(nowMs), [nowMs]);
  const heat = useMemo(() => heatmapWindow(nowMs), [nowMs]);
  const dashboard = useProviderUsageDashboard(range.startAt, range.endAt);
  const activity = useProviderUsageActivity(heat.startAt, heat.endAt);
  const budget = useApiBudgetConfig();
  const agents = useAgentUsageBreakdown(
    range.startAt,
    range.endAt,
    dimension === "project",
  );

  return useMemo(() => {
    const buckets = dashboard.data?.trendBuckets ?? [];
    const daily = dashboard.isSuccess
      ? dayPoints(buckets, range.startAt, range.endAt)
      : [];
    const points = rollupPoints(daily, granularity);
    const summary = summarize(daily);
    const capUsd = sharedCapUsd(
      budget.data?.mode,
      budget.data?.sharedDailyBudgetUsd,
    );
    const providers = dashboard.data?.providers ?? [];
    const series =
      dimension === "project"
        ? projectSeries(agents.data?.agents ?? [], unattributedLabel)
        : dimension === "client"
          ? clientSeries(providers, unattributedLabel)
          : providerSeries(providers);
    const heatResult = heatmapCells(
      activity.data ?? [],
      heat.startAt,
      heat.endAt,
      heat.maxWeeks,
    );
    return {
      maxWeeks: heatResult.maxWeeks,
      truncated: heatResult.truncated,
      weekStartsOn: WEEK_STARTS_ON_MONDAY,
      points,
      series,
      lines: dashboard.isSuccess
        ? referenceLines(points, granularity === "day" ? capUsd : null)
        : [],
      totalUsd: summary.totalUsd,
      averageUsd: summary.averageUsd,
      peakUsd: summary.peakUsd,
      capUsd,
      heatmap: heatResult.cells,
      ready: dashboard.isSuccess,
    };
  }, [
    activity.data,
    agents.data?.agents,
    budget.data?.mode,
    budget.data?.sharedDailyBudgetUsd,
    dashboard.data?.providers,
    dashboard.data?.trendBuckets,
    dashboard.isSuccess,
    dimension,
    granularity,
    heat.endAt,
    heat.maxWeeks,
    heat.startAt,
    range.endAt,
    range.startAt,
    unattributedLabel,
  ]);
}
