import { useMemo, useState } from "react";
import { useProviderUsageActivity } from "@/lib/query/providerUsageActivity";
import { useApiBudgetConfig } from "@/lib/query/trayUsage";
import { useProviderUsageDashboard } from "@/lib/query/usageDashboard";
import {
  loadPerGroupDailySeries,
  type PerGroupDailySeries,
} from "./perGroupDailyTrend";
import {
  dayPoints,
  heatmapCells,
  heatmapWindow,
  last30DaysRange,
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
} from "./trendsModel";

export interface TrendsData {
  /** C19. Always the requested window, not the tier. */
  maxWeeks: number;
  /** C19. True only when the payload is wider than `maxWeeks`. */
  truncated: boolean;
  weekStartsOn: number;
  points: CostPoint[];
  lines: TrendReferenceLine[];
  totalUsd: number | null;
  averageUsd: number | null;
  peakUsd: number | null;
  capUsd: number | null;
  heatmap: HeatCell[];
  ready: boolean;
  /** BE-16 seam. Null until real per-group daily buckets exist. */
  perGroupDaily: PerGroupDailySeries | null;
}

/**
 * Last 30 days from `get_provider_usage_dashboard` (day buckets: the window is
 * longer than 24h) and a 52-week heatmap from `get_provider_usage_activity`.
 *
 * Week and month roll up those daily totals on the client. Batch 1 does not
 * paint provider, project, or client series. `loadPerGroupDailySeries` is the
 * BE-16 seam and returns null without calling `query_usage_trend`.
 */
export function useTrendsData(
  dimension: TrendDimension,
  granularity: TrendGranularity,
): TrendsData {
  const [nowMs] = useState(() => Date.now());
  const range = useMemo(() => last30DaysRange(nowMs), [nowMs]);
  const heat = useMemo(() => heatmapWindow(nowMs), [nowMs]);
  const dashboard = useProviderUsageDashboard(range.startAt, range.endAt);
  const activity = useProviderUsageActivity(heat.startAt, heat.endAt);
  const budget = useApiBudgetConfig();
  // TODO(BE-16): plot this once it returns real daily buckets. Null keeps
  // the chart on totals. Do not pro-rate range shares in its place.
  const perGroupDaily = loadPerGroupDailySeries(dimension);

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
      lines: dashboard.isSuccess
        ? referenceLines(points, granularity === "day" ? capUsd : null)
        : [],
      totalUsd: summary.totalUsd,
      averageUsd: summary.averageUsd,
      peakUsd: summary.peakUsd,
      capUsd,
      heatmap: heatResult.cells,
      ready: dashboard.isSuccess,
      perGroupDaily,
    };
  }, [
    activity.data,
    budget.data?.mode,
    budget.data?.sharedDailyBudgetUsd,
    dashboard.data?.trendBuckets,
    dashboard.isSuccess,
    granularity,
    heat.endAt,
    heat.maxWeeks,
    heat.startAt,
    perGroupDaily,
    range.endAt,
    range.startAt,
  ]);
}
