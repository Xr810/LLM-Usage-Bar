import { describe, expect, it } from "vitest";
import { HEATMAP_MAX_WEEKS } from "@/lib/constants";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import {
  loadPerGroupDailySeries,
  QUERY_USAGE_TREND_COMMAND,
  queryUsageTrendInput,
} from "./perGroupDailyTrend";
import {
  chartScale,
  dayPoints,
  heatmapCells,
  heatmapWindow,
  last30DaysRange,
  referenceLines,
  rollupPoints,
  sharedCapUsd,
  weekStart,
  WEEK_STARTS_ON_MONDAY,
} from "./trendsModel";

function bucket(
  startAt: number,
  cost: string | null,
  endAt = startAt + 86_400,
): UsageTrendBucketView {
  return {
    startAt,
    endAt,
    eventCount: cost == null ? 0 : 1,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens: 0,
    totalCostUsd: cost,
    costSourceCounts: { upstream: 1, estimated: 0, unavailable: 0 },
  };
}

describe("trends calendar", () => {
  it("starts weeks on Monday and keeps a 30-day window", () => {
    expect(WEEK_STARTS_ON_MONDAY).toBe(1);
    const monday = new Date(2026, 9, 26);
    expect(weekStart(monday).getDate()).toBe(26);
    const sunday = new Date(2026, 9, 25);
    const start = weekStart(sunday);
    expect(start.getFullYear()).toBe(2026);
    expect(start.getMonth()).toBe(9);
    expect(start.getDate()).toBe(19);

    const now = new Date(2026, 9, 26, 15).getTime();
    const range = last30DaysRange(now);
    expect(new Date(range.startAt * 1000).getDate()).toBe(27);
    expect(new Date(range.startAt * 1000).getMonth()).toBe(8);
    const heat = heatmapWindow(now);
    expect(heat.maxWeeks).toBe(HEATMAP_MAX_WEEKS);
    expect(heat.start.getDay()).toBe(1);
    expect(heatmapCells([], heat.startAt, heat.endAt).cells).toHaveLength(
      HEATMAP_MAX_WEEKS * 7,
    );
  });

  it("rolls days into weeks and months and draws cap and max lines", () => {
    const sep30 = Math.floor(new Date(2026, 8, 30).getTime() / 1000);
    const oct1 = Math.floor(new Date(2026, 9, 1).getTime() / 1000);
    const oct2 = Math.floor(new Date(2026, 9, 2).getTime() / 1000);
    const points = dayPoints(
      [bucket(sep30, "1.00"), bucket(oct1, "2.00"), bucket(oct2, null)],
      sep30,
      oct2 + 86_400,
    );
    expect(rollupPoints(points, "month")).toHaveLength(2);
    expect(rollupPoints(points, "week").length).toBeGreaterThan(0);
    expect(
      rollupPoints(points, "month").some((point) => point.costUsd == null),
    ).toBe(true);

    const known = dayPoints([bucket(oct1, "4")], oct1, oct1 + 86_400);
    const lines = referenceLines(known, 10);
    expect(lines.map((line) => line.kind)).toEqual(["baseline", "max", "cap"]);
    expect(chartScale(lines)).toBe(10);
    expect(sharedCapUsd("shared", "10")).toBe(10);
    expect(sharedCapUsd("per_provider", "10")).toBeNull();
    expect(sharedCapUsd("shared", null)).toBeNull();
  });

  it("keeps per-group daily series unloaded until BE-16", () => {
    expect(QUERY_USAGE_TREND_COMMAND).toBe("query_usage_trend");
    expect(queryUsageTrendInput("provider")).toEqual({
      granularity: "day",
      groupBy: "provider",
    });
    expect(queryUsageTrendInput("project")).toEqual({
      granularity: "day",
      groupBy: "project",
    });
    expect(queryUsageTrendInput("client")).toEqual({
      granularity: "day",
      groupBy: "client",
    });
    expect(loadPerGroupDailySeries("provider")).toBeNull();
    expect(loadPerGroupDailySeries("project")).toBeNull();
    expect(loadPerGroupDailySeries("client")).toBeNull();
  });

  it("marks the heatmap truncated when a bucket falls outside the window", () => {
    const heat = heatmapWindow(new Date(2026, 9, 26).getTime());
    const inside = heatmapCells(
      [bucket(heat.startAt, "1")],
      heat.startAt,
      heat.endAt,
    );
    expect(inside.truncated).toBe(false);
    expect(inside.maxWeeks).toBe(52);
    const outside = heatmapCells(
      [bucket(heat.startAt - 86_400, "1")],
      heat.startAt,
      heat.endAt,
    );
    expect(outside.truncated).toBe(true);
  });
});
