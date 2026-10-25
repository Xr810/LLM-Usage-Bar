import type { ReactNode } from "react";
import { barFill } from "./capMath";

interface CapBarProps {
  value: number;
  cap: number | null;
  scale: "cap" | "raw";
  variant: "day" | "thin" | "row";
  /** Accent is the only progress color. Muted is the stopped fill. */
  tone: "accent" | "muted";
  /**
   * Near the alert threshold. Changes the accessible name and tooltip only.
   * The fill stays `tone`.
   */
  near?: boolean;
  /** Native tooltip. Used when `near` is set. */
  tooltip?: string;
  label: string;
  valueText: string;
  marker?: ReactNode;
}

export function CapBar({
  value,
  cap,
  scale,
  variant,
  tone,
  near = false,
  tooltip,
  label,
  valueText,
  marker,
}: CapBarProps) {
  const fill = barFill(value, cap, scale);
  const hasCap = cap != null && cap > 0;
  return (
    <div
      className={
        variant === "day"
          ? "lub-meter"
          : variant === "thin"
            ? "lub-meter lub-meter-thin"
            : "lub-meter lub-meter-row"
      }
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={hasCap ? cap : undefined}
      aria-valuenow={hasCap ? Math.min(value, cap) : 0}
      aria-valuetext={valueText}
      data-near={near ? "true" : undefined}
      title={near ? tooltip : undefined}
    >
      <span
        className={
          tone === "muted"
            ? "lub-meter-fill lub-meter-fill-muted"
            : variant === "thin"
              ? "lub-meter-fill lub-meter-fill-month"
              : "lub-meter-fill"
        }
        style={{ width: `${fill.fillPercent}%` }}
      />
      {fill.overflowPercent > 0 ? (
        <span
          className="lub-meter-overflow"
          style={{
            left: `${fill.fillPercent}%`,
            width: `${fill.overflowPercent}%`,
          }}
        />
      ) : null}
      {marker}
    </div>
  );
}
