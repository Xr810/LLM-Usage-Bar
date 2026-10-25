import { lubBar } from "@/styles/lubTokens";

/** Cap line sits at 1 / scale of the track (83.3% when scale is 1.2). */
export const CAP_LINE_RATIO = 1 / lubBar.scale;

export function parseNonNegative(
  value: string | null | undefined,
): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

export interface BarFill {
  /** Accent (or warn) segment width, 0–100. */
  fillPercent: number;
  /** Stop-colored overflow past the cap line, 0–100. */
  overflowPercent: number;
}

/**
 * Dollar bars share the total-card scale: the track maximum is cap × 1.2,
 * so a full cap lands on the same 83.3% mark as the vertical line.
 * Raw bars (subscription percent, session spent/cap) use the value itself.
 */
export function barFill(
  value: number,
  cap: number | null,
  scale: "cap" | "raw",
): BarFill {
  if (!Number.isFinite(value) || value < 0) {
    return { fillPercent: 0, overflowPercent: 0 };
  }
  if (scale === "raw") {
    const ratio = cap != null && cap > 0 ? value / cap : value;
    return {
      fillPercent: Math.min(100, Math.max(0, ratio * 100)),
      overflowPercent: 0,
    };
  }
  if (cap == null || cap <= 0) {
    return { fillPercent: 0, overflowPercent: 0 };
  }
  const max = cap * lubBar.scale;
  const ratio = value / max;
  const capped = Math.min(ratio, CAP_LINE_RATIO);
  const overflow = Math.max(0, Math.min(ratio, 1) - CAP_LINE_RATIO);
  return {
    fillPercent: capped * 100,
    overflowPercent: overflow * 100,
  };
}

export function reachesWarn(
  value: number,
  cap: number,
  thresholdPct: number,
): boolean {
  if (cap <= 0) return false;
  return (value / cap) * 100 >= thresholdPct;
}
