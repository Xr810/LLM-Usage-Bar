import { describe, expect, it } from "vitest";
import { CAP_LINE_RATIO, barFill, reachesWarn } from "./capMath";

describe("cap bar math", () => {
  it("places the single cap line at 83.3% of the track", () => {
    expect(CAP_LINE_RATIO * 100).toBeCloseTo(83.3, 1);
  });

  it("scales dollar fills so a full cap lands on that line", () => {
    expect(barFill(7.42, 10, "cap").fillPercent).toBeCloseTo(61.83, 1);
    expect(barFill(5.1, 10, "cap").fillPercent).toBeCloseTo(42.5, 1);
    expect(barFill(2.32, 10, "cap").fillPercent).toBeCloseTo(19.33, 1);
    expect(barFill(3, 10, "cap").fillPercent).toBeCloseTo(25, 1);
  });

  it("keeps session and percent rows on a raw scale", () => {
    expect(barFill(3.8, 5, "raw").fillPercent).toBeCloseTo(76, 1);
    expect(barFill(62, 100, "raw").fillPercent).toBeCloseTo(62, 1);
  });

  it("warns a session only at the alert threshold", () => {
    expect(reachesWarn(3.8, 5, 80)).toBe(false);
    expect(reachesWarn(4, 5, 80)).toBe(true);
  });
});
