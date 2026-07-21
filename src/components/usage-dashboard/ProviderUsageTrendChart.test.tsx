import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import { ProviderUsageTrendChart } from "./ProviderUsageTrendChart";

function bucket(
  startAt: number,
  totalTokens: number,
  eventCount: number,
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
    totalCostUsd: null,
    costSourceCounts: { upstream: 0, estimated: 0, unavailable: eventCount },
  };
}

describe("ProviderUsageTrendChart", () => {
  it("presents one total-token trend with peak and record semantics", () => {
    render(
      <ProviderUsageTrendChart
        granularity="day"
        buckets={[
          bucket(1_720_000_000, 1_500, 2),
          bucket(1_720_086_400, 5_000, 3),
        ]}
        totalTokens={6_500}
        recordCount={5}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Usage trend" }),
    ).toBeInTheDocument();
    expect(screen.getByText("6.5K")).toBeInTheDocument();
    expect(screen.getByText(/Peak 5K/)).toHaveTextContent("5 records");
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
        recordCount={0}
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
