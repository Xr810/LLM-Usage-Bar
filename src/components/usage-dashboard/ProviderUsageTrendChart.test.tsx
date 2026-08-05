import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import {
  ProviderUsageTrendChart,
  ProviderUsageTrendTooltip,
} from "./ProviderUsageTrendChart";

function bucket(
  startAt: number,
  totalTokens: number,
  eventCount: number,
  totalCostUsd: string | null = null,
): UsageTrendBucketView {
  return {
    startAt,
    endAt: startAt + 24 * 60 * 60,
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

describe("ProviderUsageTrendChart", () => {
  it("pairs compact token units with what the day cost", () => {
    const point = {
      ...bucket(1_720_000_000, 368_144_526, 1_526, "412.5"),
      label: "7/19",
      tooltipLabel: "July 19, 2026",
    };

    render(<ProviderUsageTrendTooltip active payload={[{ payload: point }]} />);

    expect(screen.getByText("July 19, 2026")).toBeInTheDocument();
    expect(screen.getByText(/368\.1M Token/)).toHaveTextContent(
      "368.1M Token · $412.50",
    );
    expect(screen.queryByText(/368,144,526/)).toBeNull();
    expect(screen.queryByText(/1526/)).toBeNull();
  });

  it("says an unpriced bucket is unpriced rather than showing it as free", () => {
    const point = {
      ...bucket(1_720_000_000, 1_000, 2),
      label: "7/19",
      tooltipLabel: "July 19, 2026",
    };

    render(<ProviderUsageTrendTooltip active payload={[{ payload: point }]} />);

    expect(screen.getByText(/1K Token/)).toHaveTextContent("Cost unavailable");
    expect(screen.queryByText(/\$0/)).toBeNull();
  });

  it("presents one total-token trend with peak and spend semantics", () => {
    render(
      <ProviderUsageTrendChart
        granularity="day"
        buckets={[
          bucket(1_720_000_000, 1_500, 2),
          bucket(1_720_086_400, 5_000, 3),
        ]}
        totalTokens={6_500}
        totalCostUsd="18.25"
        rangeLabel="30 days"
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Usage trend" }),
    ).toBeInTheDocument();
    expect(screen.getByText("6.5K")).toBeInTheDocument();
    expect(screen.getByTestId("usage-trend-range")).toHaveTextContent(
      "30 days",
    );
    expect(screen.getByText(/Peak 5K/)).toHaveTextContent("$18.25");
    expect(
      screen.getByRole("img", { name: "Token usage by day" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Cache creation")).toBeNull();
  });

  it("shows an empty state when every bucket is zero", () => {
    render(
      <ProviderUsageTrendChart
        granularity="hour"
        buckets={[bucket(1_720_000_000, 0, 0)]}
        totalTokens={0}
        totalCostUsd="0"
        rangeLabel="Today"
      />,
    );

    expect(
      screen.getByText("No usage recorded in this range."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("img", { name: "Token usage by hour" }),
    ).toBeNull();
  });
});
