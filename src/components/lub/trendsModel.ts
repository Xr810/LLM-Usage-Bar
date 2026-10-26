import { HEATMAP_MAX_WEEKS } from "@/lib/constants";
import {
  buildProviderActivityDays,
  type ProviderActivityDay,
} from "@/components/usage-dashboard/ProviderActivityHeatmap";
import type { UsageTrendBucketView } from "@/types/usageDashboard";

/** Monday. Heatmap rows and week buckets both start here. */
export const WEEK_STARTS_ON_MONDAY = 1;

export const UPSELL_ENTRY_FEATURE = "upsell_entry";
export const WEEKDAY_HOUR_FEATURE = "heatmap_weekday_hour";

export type TrendGranularity = "day" | "week" | "month";
export type TrendDimension = "provider" | "project" | "client";

export interface CostPoint {
  /** Local midnight, unix seconds. */
  startAt: number;
  /** Null when a bucket in the group was never priced. */
  costUsd: number | null;
}

export interface TrendReferenceLine {
  kind: "baseline" | "max" | "cap";
  amountUsd: number;
}

export interface HeatCell {
  key: string;
  week: number;
  weekday: number;
  level: 0 | 1 | 2 | 3 | 4;
}

export function showsWeekdayHour(features: readonly string[]): boolean {
  return (
    features.includes(UPSELL_ENTRY_FEATURE) ||
    features.includes(WEEKDAY_HOUR_FEATURE)
  );
}

/** Local midnight of the week that contains `date`. Default week starts Monday. */
export function weekStart(
  date: Date,
  weekStartsOn = WEEK_STARTS_ON_MONDAY,
): Date {
  const start = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const delta = (start.getDay() - weekStartsOn + 7) % 7;
  start.setDate(start.getDate() - delta);
  return start;
}

function startOfLocalDay(ms: number): Date {
  const date = new Date(ms);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** Half-open local window: today and the 29 days before it. */
export function last30DaysRange(nowMs: number): {
  startAt: number;
  endAt: number;
} {
  const start = startOfLocalDay(nowMs);
  start.setDate(start.getDate() - 29);
  return {
    startAt: Math.floor(start.getTime() / 1000),
    endAt: Math.floor(nowMs / 1000) + 1,
  };
}

/**
 * Exactly `maxWeeks` Monday-start weeks ending with the current week.
 * `endAt` is the next Monday, exclusive.
 */
export function heatmapWindow(
  nowMs: number,
  maxWeeks = HEATMAP_MAX_WEEKS,
): { startAt: number; endAt: number; maxWeeks: number; start: Date } {
  const currentWeek = weekStart(startOfLocalDay(nowMs));
  const start = new Date(currentWeek);
  start.setDate(start.getDate() - (maxWeeks - 1) * 7);
  const end = new Date(currentWeek);
  end.setDate(end.getDate() + 7);
  return {
    startAt: Math.floor(start.getTime() / 1000),
    endAt: Math.floor(end.getTime() / 1000),
    maxWeeks,
    start,
  };
}

function parseUsd(value: string | null | undefined): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

function addCost(left: number | null, right: number | null): number | null {
  if (left == null || right == null) return null;
  return left + right;
}

/**
 * One point per local day in `[startAt, endAt)`. Days the dashboard did not
 * return cost nothing. A bucket with a null amount makes that day unknown.
 */
export function dayPoints(
  buckets: UsageTrendBucketView[],
  startAt: number,
  endAt: number,
): CostPoint[] {
  const byDay = new Map<number, number | null>();
  for (const bucket of buckets) {
    const day = Math.floor(
      startOfLocalDay(bucket.startAt * 1000).getTime() / 1000,
    );
    const cost = parseUsd(bucket.totalCostUsd);
    const previous = byDay.get(day);
    byDay.set(day, previous === undefined ? cost : addCost(previous, cost));
  }

  const points: CostPoint[] = [];
  for (
    let cursor = startOfLocalDay(startAt * 1000);
    cursor.getTime() / 1000 < endAt;
    cursor = new Date(
      cursor.getFullYear(),
      cursor.getMonth(),
      cursor.getDate() + 1,
    )
  ) {
    const key = Math.floor(cursor.getTime() / 1000);
    const cost = byDay.get(key);
    points.push({ startAt: key, costUsd: cost === undefined ? 0 : cost });
  }
  return points;
}

export function rollupPoints(
  points: CostPoint[],
  granularity: TrendGranularity,
): CostPoint[] {
  if (granularity === "day") return points;
  const groups = new Map<number, number | null>();
  for (const point of points) {
    const date = new Date(point.startAt * 1000);
    const bucket =
      granularity === "week"
        ? weekStart(date)
        : new Date(date.getFullYear(), date.getMonth(), 1);
    const key = Math.floor(bucket.getTime() / 1000);
    const previous = groups.get(key);
    groups.set(
      key,
      previous === undefined ? point.costUsd : addCost(previous, point.costUsd),
    );
  }
  return [...groups.entries()]
    .sort((left, right) => left[0] - right[0])
    .map(([startAt, costUsd]) => ({ startAt, costUsd }));
}

export function summarize(points: CostPoint[]): {
  totalUsd: number | null;
  averageUsd: number | null;
  peakUsd: number | null;
} {
  if (points.length === 0 || points.some((point) => point.costUsd == null)) {
    return { totalUsd: null, averageUsd: null, peakUsd: null };
  }
  const totalUsd = points.reduce((sum, point) => sum + (point.costUsd ?? 0), 0);
  const peakUsd = points.reduce(
    (peak, point) => Math.max(peak, point.costUsd ?? 0),
    0,
  );
  return { totalUsd, averageUsd: totalUsd / points.length, peakUsd };
}

/**
 * Shared daily cap only. Per-provider mode has no single line to draw.
 */
export function sharedCapUsd(
  mode: string | undefined,
  sharedDailyBudgetUsd: string | null | undefined,
): number | null {
  if (mode !== "shared") return null;
  const cap = parseUsd(sharedDailyBudgetUsd);
  return cap != null && cap > 0 ? cap : null;
}

/** Baseline at $0, a max line at the peak, and the daily cap when one is set. */
export function referenceLines(
  points: CostPoint[],
  capUsd: number | null,
): TrendReferenceLine[] {
  const known = points
    .map((point) => point.costUsd)
    .filter((cost): cost is number => cost != null);
  const peak = known.length === 0 ? 0 : Math.max(...known);
  const lines: TrendReferenceLine[] = [
    { kind: "baseline", amountUsd: 0 },
    { kind: "max", amountUsd: peak },
  ];
  if (capUsd != null) lines.push({ kind: "cap", amountUsd: capUsd });
  return lines;
}

export function chartScale(lines: TrendReferenceLine[]): number {
  const top = lines.reduce((peak, line) => Math.max(peak, line.amountUsd), 0);
  return top > 0 ? top : 1;
}

function spendLevel(
  day: ProviderActivityDay,
  maxCost: number,
): 0 | 1 | 2 | 3 | 4 {
  const cost = parseUsd(day.totalCostUsd);
  const active = day.eventCount > 0 || (cost != null && cost > 0);
  if (!active) return 0;
  if (cost == null || maxCost <= 0) return 1;
  if (cost <= 0) return 1;
  return Math.max(1, Math.min(4, Math.ceil(Math.sqrt(cost / maxCost) * 4))) as
    1 | 2 | 3 | 4;
}

/**
 * 52 Monday-start columns. `truncated` is the C19 flag: true when a returned
 * bucket falls outside the requested window. The client asks for exactly
 * `maxWeeks`, so this stays false until the payload is wider than that.
 */
export function heatmapCells(
  buckets: UsageTrendBucketView[],
  startAt: number,
  endAt: number,
  maxWeeks = HEATMAP_MAX_WEEKS,
): { cells: HeatCell[]; truncated: boolean; maxWeeks: number } {
  const days = buildProviderActivityDays(buckets, startAt, endAt);
  const costs = days.map((day) => parseUsd(day.totalCostUsd) ?? 0);
  const maxCost = costs.reduce((peak, cost) => Math.max(peak, cost), 0);
  const expected = maxWeeks * 7;
  const cells: HeatCell[] = Array.from({ length: expected }, (_, index) => {
    const day = days[index];
    return {
      key: day?.key ?? `empty-${index}`,
      week: Math.floor(index / 7),
      weekday: index % 7,
      level: day ? spendLevel(day, maxCost) : 0,
    };
  });
  const truncated =
    days.length > expected ||
    buckets.some((bucket) => bucket.startAt < startAt || bucket.endAt > endAt);
  return { cells, truncated, maxWeeks };
}

export function weekdayLabels(
  locale: string,
  weekStartsOn = WEEK_STARTS_ON_MONDAY,
): string[] {
  const start = weekStart(new Date(2026, 9, 26), weekStartsOn);
  const formatter = new Intl.DateTimeFormat(locale, { weekday: "short" });
  return Array.from({ length: 7 }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    return formatter.format(date);
  });
}

export function axisLabels(
  points: CostPoint[],
  locale: string,
): { startAt: number; text: string; iso: string }[] {
  const step = points.length <= 8 ? 1 : Math.ceil((points.length - 1) / 5);
  const formatter = new Intl.DateTimeFormat(locale, {
    month: "numeric",
    day: "numeric",
  });
  return points.map((point, index) => {
    const date = new Date(point.startAt * 1000);
    const show =
      index === 0 || index === points.length - 1 || index % step === 0;
    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    return {
      startAt: point.startAt,
      text: show ? formatter.format(date) : "",
      iso: `${date.getFullYear()}-${month}-${day}`,
    };
  });
}
