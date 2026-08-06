import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
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
  totalCostUsd: string | null = null,
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
    totalCostUsd,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: eventCount },
  };
}

describe("ProviderActivityHeatmap", () => {
  const startAt = timestamp(2026, 6, 1);
  const endAt = timestamp(2026, 6, 4);

  afterEach(() => vi.restoreAllMocks());

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

  it("sums a day's spend, and drops it entirely when one bucket was never priced", () => {
    const days = buildProviderActivityDays(
      [
        bucket(timestamp(2026, 6, 1, 9), 100, 1, "1.25"),
        bucket(timestamp(2026, 6, 1, 18), 300, 2, "0.75"),
        // The unpriced bucket is what makes 7/3 unknown: reporting only its
        // priced neighbour would understate the day as if the rest were free.
        bucket(timestamp(2026, 6, 3, 8), 1_000, 1, "9"),
        bucket(timestamp(2026, 6, 3, 12), 4_000, 4, null),
      ],
      startAt,
      endAt,
    );

    expect(days[0]).toMatchObject({ totalTokens: 400, totalCostUsd: "2" });
    expect(days[1]).toMatchObject({ totalTokens: 0, totalCostUsd: "0" });
    expect(days[2]).toMatchObject({ totalTokens: 5_000, totalCostUsd: null });
  });

  it("renders one accessible cell per day with Token and spend details", () => {
    render(
      <ProviderActivityHeatmap
        buckets={[
          bucket(timestamp(2026, 6, 1, 9), 1_250, 2, "3.5"),
          bucket(timestamp(2026, 6, 3, 9), 9_500, 6, "12"),
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
    expect(cells[0]).toHaveAccessibleName(/1,250 Token · \$3\.50/);
    expect(cells[1]).toHaveAttribute("data-activity-level", "0");
    expect(cells[2]).toHaveAttribute("data-activity-level", "4");
    // Nothing is pinnable, so nothing is a toggle.
    expect(cells[0]).not.toHaveAttribute("aria-pressed");
    expect(
      cells[0]!.querySelector("[data-activity-cell-visual]"),
    ).toBeInTheDocument();
    expect(cells.filter((cell) => cell.tabIndex === 0)).toEqual([cells[2]]);
    expect(screen.queryByText(/2026.*7.*3/)).toBeNull();
    expect(screen.getByText("Hover a day for its detail")).toBeVisible();

    fireEvent.mouseEnter(cells[0]!);
    expect(screen.getByText(/2026.*7.*1/)).toBeInTheDocument();
    expect(screen.getByText(/1.3K Token · \$3\.50/)).toBeInTheDocument();
    expect(screen.queryByText(/messages/i)).toBeNull();
    fireEvent.mouseLeave(cells[0]!);
    expect(screen.queryByText(/2026.*7.*1/)).toBeNull();

    fireEvent.mouseEnter(cells[1]!);
    expect(screen.getByText(/2026.*7.*2/)).toBeInTheDocument();
    // A day nothing happened on cost $0 — that is known, not missing.
    expect(screen.getByText("0 Token · $0.00")).toBeInTheDocument();
    fireEvent.mouseLeave(cells[1]!);

    // Clicking must not leave the panel sitting on a day the pointer left.
    fireEvent.click(cells[0]!);
    expect(screen.queryByText(/2026.*7.*1/)).toBeNull();
    fireEvent.mouseEnter(cells[0]!);
    fireEvent.mouseLeave(cells[0]!);
    expect(screen.queryByText(/2026.*7.*1/)).toBeNull();

    act(() => cells[2]!.focus());
    expect(screen.getByText(/2026.*7.*3/)).toBeInTheDocument();
    fireEvent.keyDown(cells[2]!, { key: "ArrowUp" });
    expect(cells[1]).toHaveFocus();
    expect(cells.filter((cell) => cell.tabIndex === 0)).toEqual([cells[1]]);
    act(() => cells[1]!.blur());
    expect(screen.queryByText(/2026.*7.*2/)).toBeNull();
  });

  it("starts at the latest dates without overwriting later user scrolling", () => {
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(720);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(600);

    const { rerender } = render(
      <ProviderActivityHeatmap
        buckets={[bucket(timestamp(2026, 6, 3, 9), 9_500, 6)]}
        startAt={startAt}
        endAt={endAt}
      />,
    );
    const viewport = document.querySelector<HTMLElement>(
      "[data-activity-scroll]",
    );

    expect(viewport).not.toBeNull();
    expect(viewport?.scrollLeft).toBe(120);

    if (!viewport) throw new Error("activity scroll viewport missing");
    viewport.scrollLeft = 48;
    rerender(
      <ProviderActivityHeatmap
        buckets={[bucket(timestamp(2026, 6, 2, 9), 4_200, 3)]}
        startAt={startAt}
        endAt={endAt}
      />,
    );

    expect(viewport.scrollLeft).toBe(48);
  });
});
