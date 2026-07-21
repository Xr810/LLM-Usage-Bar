import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import {
  buildProviderActivityDays,
  ProviderActivityHeatmap,
} from "./ProviderActivityHeatmap";

function timestamp(year: number, month: number, day: number, hour = 0) {
  return Math.floor(new Date(year, month, day, hour).getTime() / 1_000);
}

function bucket(
  startAt: number,
  totalTokens: number,
  eventCount: number,
): UsageTrendBucketView {
  return {
    startAt,
    endAt: startAt + 60 * 60,
    eventCount,
    inputTokens: totalTokens,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens,
    totalCostUsd: null,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: eventCount },
  };
}

describe("ProviderActivityHeatmap", () => {
  const startAt = timestamp(2026, 6, 1);
  const endAt = timestamp(2026, 6, 4);

  it("merges hourly buckets into local days and fills missing dates", () => {
    const days = buildProviderActivityDays(
      [
        bucket(timestamp(2026, 6, 1, 9), 100, 1),
        bucket(timestamp(2026, 6, 1, 18), 300, 2),
        bucket(timestamp(2026, 6, 3, 12), 4_000, 4),
      ],
      startAt,
      endAt,
    );

    expect(days).toHaveLength(3);
    expect(days[0]).toMatchObject({ eventCount: 3, totalTokens: 400 });
    expect(days[0]).toHaveProperty("level", 2);
    expect(days[1]).toMatchObject({
      eventCount: 0,
      totalTokens: 0,
      level: 0,
    });
    expect(days[2]).toMatchObject({
      eventCount: 4,
      totalTokens: 4_000,
      level: 4,
    });
  });

  it("renders one accessible cell per day with Token and record details", () => {
    render(
      <ProviderActivityHeatmap
        buckets={[
          bucket(timestamp(2026, 6, 1, 9), 1_250, 2),
          bucket(timestamp(2026, 6, 3, 9), 9_500, 6),
        ]}
        startAt={startAt}
        endAt={endAt}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Daily activity" }),
    ).toBeInTheDocument();
    expect(screen.getByText("2 active days")).toBeInTheDocument();
    const cells = screen.getAllByRole("button");
    expect(cells).toHaveLength(3);
    expect(cells[0]).toHaveAccessibleName(/1,250 Token · 2 records/);
    expect(cells[1]).toHaveAttribute("data-activity-level", "0");
    expect(cells[2]).toHaveAttribute("data-activity-level", "4");
    expect(cells.filter((cell) => cell.tabIndex === 0)).toEqual([cells[2]]);

    fireEvent.mouseEnter(cells[0]!);
    expect(screen.getByText(/1.3K Token · 2 records/)).toBeInTheDocument();
    expect(screen.queryByText(/messages/i)).toBeNull();

    cells[2]!.focus();
    fireEvent.keyDown(cells[2]!, { key: "ArrowUp" });
    expect(cells[1]).toHaveFocus();
    expect(cells.filter((cell) => cell.tabIndex === 0)).toEqual([cells[1]]);
  });
});
