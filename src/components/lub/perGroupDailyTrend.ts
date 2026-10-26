import type { TrendDimension } from "./trendsModel";

/** BE-16 command. Batch 1 must not invoke it. */
export const QUERY_USAGE_TREND_COMMAND = "query_usage_trend";

export interface QueryUsageTrendInput {
  granularity: "day";
  groupBy: TrendDimension;
}

/**
 * Arguments for real per-group daily bars.
 * `groupBy` is `provider`, `project`, or `client`.
 */
export function queryUsageTrendInput(
  groupBy: TrendDimension,
): QueryUsageTrendInput {
  return { granularity: "day", groupBy };
}

/**
 * Real per-group daily buckets. Batch 1 never builds one.
 * Project and client toggles render only when `loadPerGroupDailySeries`
 * returns a series, so they appear with the BE-16 payload and need no
 * second flag.
 */
export interface PerGroupDailySeries {
  groupBy: TrendDimension;
}

/**
 * Batch-2 seam (BE-16).
 *
 * Real per-group daily bars come from
 * `query_usage_trend({ granularity: 'day', groupBy })`.
 * Batch 1 returns null and the page draws dashboard totals only.
 * Do not split those totals by a group's share of the range: an
 * estimate would look like measured days.
 *
 * TODO(BE-16): invoke `QUERY_USAGE_TREND_COMMAND` with
 * `queryUsageTrendInput(groupBy)` and map the buckets into daily
 * series. Do not call the command until that payload exists.
 */
export function loadPerGroupDailySeries(
  _groupBy: TrendDimension,
): PerGroupDailySeries | null {
  return null;
}
