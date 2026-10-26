import { describe, expect, it } from "vitest";
import {
  clampToFreeWindow,
  resolveProviderActivityRange,
  resolveUsageRange,
} from "./usageRange";

describe("resolveUsageRange", () => {
  const nowMs = new Date("2026-07-12T12:34:56.789Z").getTime();
  const currentSecond = Math.floor(nowMs / 1000);

  it.each(["today", "1d", "7d", "14d", "30d", "1y"] as const)(
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

  it("uses 365 local calendar days for the 1 year preset", () => {
    const now = new Date(2026, 6, 21, 18, 30, 45);
    const range = resolveUsageRange({ preset: "1y" }, now.getTime());
    const start = new Date(range.startDate * 1_000);

    expect([
      start.getFullYear(),
      start.getMonth(),
      start.getDate(),
      start.getHours(),
    ]).toEqual([2025, 6, 22, 0]);
    expect(range.endDate).toBe(Math.floor(now.getTime() / 1_000) + 1);
  });

  it("keeps custom ranges longer than one year unchanged", () => {
    const startDate = Math.floor(
      new Date(2024, 0, 1, 0, 0, 0).getTime() / 1_000,
    );
    const endDate = Math.floor(
      new Date(2026, 6, 21, 18, 30, 0).getTime() / 1_000,
    );

    expect(
      resolveUsageRange(
        {
          preset: "custom",
          customStartDate: startDate,
          customEndDate: endDate,
          liveEndTime: false,
        },
        nowMs,
      ),
    ).toEqual({ startDate, endDate });
  });
});

describe("clampToFreeWindow", () => {
  const now = new Date(2026, 2, 20, 12, 0, 0);
  const nowMs = now.getTime();

  it("turns 1y into 30d and leaves the short presets unchanged", () => {
    expect(clampToFreeWindow({ preset: "1y" }, nowMs)).toEqual({
      preset: "30d",
    });
    for (const preset of ["today", "1d", "7d", "14d", "30d"] as const) {
      const selection = { preset };
      expect(clampToFreeWindow(selection, nowMs)).toBe(selection);
    }
  });

  it("clamps a custom start across a month boundary to local midnight", () => {
    const marchFirst = new Date(2026, 2, 1, 15, 30, 45);
    const tooEarly = Math.floor(
      new Date(2026, 0, 1, 12, 0, 0).getTime() / 1000,
    );
    const end = Math.floor(marchFirst.getTime() / 1000);
    const selection = {
      preset: "custom" as const,
      customStartDate: tooEarly,
      customEndDate: end,
      liveEndTime: false,
    };
    const clamped = clampToFreeWindow(selection, marchFirst.getTime());
    const start = new Date((clamped.customStartDate ?? 0) * 1000);

    expect(clamped.preset).toBe("custom");
    expect(clamped.customEndDate).toBe(end);
    expect(clamped.liveEndTime).toBe(false);
    // March 1 minus 29 days is January 31.
    expect([
      start.getFullYear(),
      start.getMonth(),
      start.getDate(),
      start.getHours(),
      start.getMinutes(),
      start.getSeconds(),
    ]).toEqual([2026, 0, 31, 0, 0, 0]);
  });

  it("clamps to local midnight across a DST gap, not 29 times 24 hours", () => {
    const tooEarly = Math.floor(
      new Date(2026, 0, 15, 3, 30, 0).getTime() / 1000,
    );
    const clamped = clampToFreeWindow(
      {
        preset: "custom",
        customStartDate: tooEarly,
        customEndDate: Math.floor(nowMs / 1000),
        liveEndTime: true,
      },
      nowMs,
    );
    const start = new Date((clamped.customStartDate ?? 0) * 1000);
    const naive = new Date(nowMs - 29 * 24 * 60 * 60 * 1000);

    // March 20 minus 29 calendar days is February 19 at local midnight.
    expect([
      start.getFullYear(),
      start.getMonth(),
      start.getDate(),
      start.getHours(),
      start.getMinutes(),
      start.getSeconds(),
    ]).toEqual([2026, 1, 19, 0, 0, 0]);
    expect(clamped.liveEndTime).toBe(true);
    expect(start.getTime()).not.toBe(naive.getTime());
  });

  it("leaves a custom range inside the 30-day window unchanged", () => {
    const start = Math.floor(new Date(2026, 2, 10, 8, 15, 0).getTime() / 1000);
    const boundary = Math.floor(
      new Date(2026, 1, 19, 0, 0, 0).getTime() / 1000,
    );
    const inside = {
      preset: "custom" as const,
      customStartDate: start,
      customEndDate: Math.floor(nowMs / 1000),
      liveEndTime: false,
    };
    const onBoundary = { ...inside, customStartDate: boundary };

    expect(clampToFreeWindow(inside, nowMs)).toBe(inside);
    expect(clampToFreeWindow(onBoundary, nowMs)).toBe(onBoundary);
    expect(
      clampToFreeWindow({ ...inside, customStartDate: boundary - 1 }, nowMs)
        .customStartDate,
    ).toBe(boundary);
  });
});

describe("resolveProviderActivityRange", () => {
  it("returns 365 local calendar days with stable midnight boundaries", () => {
    const now = new Date(2026, 6, 21, 18, 30, 45);
    const range = resolveProviderActivityRange(now.getTime());
    const start = new Date(range.startDate * 1_000);
    const end = new Date(range.endDate * 1_000);

    expect([
      start.getFullYear(),
      start.getMonth(),
      start.getDate(),
      start.getHours(),
    ]).toEqual([2025, 6, 22, 0]);
    expect([
      end.getFullYear(),
      end.getMonth(),
      end.getDate(),
      end.getHours(),
    ]).toEqual([2026, 6, 22, 0]);
  });
});
