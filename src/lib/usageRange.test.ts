import { describe, expect, it } from "vitest";
import { resolveUsageRange } from "./usageRange";

describe("resolveUsageRange", () => {
  const nowMs = new Date("2026-07-12T12:34:56.789Z").getTime();
  const currentSecond = Math.floor(nowMs / 1000);

  it.each(["today", "1d", "7d", "14d", "30d"] as const)(
    "uses a half-open live end for %s so events in the current second are included",
    (preset) => {
      expect(resolveUsageRange({ preset }, nowMs).endDate).toBe(
        currentSecond + 1,
      );
    },
  );

  it("advances a live custom end but keeps a fixed custom end unchanged", () => {
    expect(
      resolveUsageRange(
        {
          preset: "custom",
          customStartDate: 100,
          customEndDate: 200,
          liveEndTime: true,
        },
        nowMs,
      ),
    ).toEqual({ startDate: 100, endDate: currentSecond + 1 });

    expect(
      resolveUsageRange(
        {
          preset: "custom",
          customStartDate: 100,
          customEndDate: 200,
          liveEndTime: false,
        },
        nowMs,
      ),
    ).toEqual({ startDate: 100, endDate: 200 });
  });
});
