import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import fixture from "../../../tests/fixtures/usage-trend-contract.json";
import type { UsageTrendRequest, UsageTrendResponse } from "@/types/usage";
import { usageApi } from "./usage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("daily trend contract (shared with Rust query/serialization)", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("sends daily grouping and filters and preserves absent versus zero cost", async () => {
    const request: UsageTrendRequest = {
      ...fixture.request,
      granularity: "day",
      groupBy: "project",
    };
    const expected: UsageTrendResponse = fixture.response;
    vi.mocked(invoke).mockResolvedValueOnce(expected);
    const response = await usageApi.queryUsageTrend(request);
    expect(invoke).toHaveBeenCalledWith("query_usage_trend", request);
    expect(response).toEqual(expected);
    expect(response.series[0].points[1]).not.toHaveProperty("costMicroUsd");
    expect(response.series[0].points[2].costMicroUsd).toBe(0);
    expect(response.series[1].groupLabel).toBe("");
    expect(response).not.toHaveProperty("clampedFrom");
  });

  it("preserves clamping metadata and coded authorization errors", async () => {
    const request: UsageTrendRequest = {
      tz: "Asia/Hong_Kong",
      from: "2020-01-01",
      to: "2020-01-02",
      granularity: "day",
    };
    const response: UsageTrendResponse = {
      series: [{ groupKey: "all", groupLabel: "", points: [] }],
      clampedFrom: "2020-01-01",
    };
    vi.mocked(invoke).mockResolvedValueOnce(response);
    expect(await usageApi.queryUsageTrend(request)).toEqual(response);
    const error = {
      code: "PRO_REQUIRED",
      message: "Grouped usage trends require Pro.",
    };
    vi.mocked(invoke).mockRejectedValueOnce(error);
    await expect(
      usageApi.queryUsageTrend({ ...request, groupBy: "provider" }),
    ).rejects.toEqual(error);
  });
});
