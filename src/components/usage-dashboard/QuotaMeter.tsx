import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import type { UsageTone } from "./usagePresentation";

const TONE_BAR: Record<UsageTone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-danger",
  muted: "bg-muted-foreground/40",
};

interface QuotaMeterProps {
  /** Window label, e.g. "5 小时额度" */
  label: string;
  /** Primary figure shown right-aligned, e.g. "剩余 63%" */
  valueText: string;
  /** 0–100 fill of the bar; null hides the bar */
  fillPercent: number | null;
  tone: UsageTone;
  /** Accessibility name for the meter */
  meterLabel: string;
  /** Small muted line under the bar (reset countdown etc.) */
  footer?: ReactNode;
}

/**
 * Compact quota meter: label + remaining figure, a thin status-colored bar,
 * and a small caption line. Used by subscription Provider cards.
 */
export function QuotaMeter({
  label,
  valueText,
  fillPercent,
  tone,
  meterLabel,
  footer,
}: QuotaMeterProps) {
  return (
    <div className="rounded-lg border border-border/50 bg-muted/25 px-3 py-2.5 dark:bg-muted/15">
      <div className="flex items-baseline justify-between gap-2">
        <span className="truncate text-xs text-muted-foreground">{label}</span>
        <span className="shrink-0 text-sm font-semibold tracking-tight metric">
          {valueText}
        </span>
      </div>
      {fillPercent != null ? (
        <div
          role="progressbar"
          aria-label={meterLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(fillPercent)}
          className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
        >
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-500 ease-out",
              TONE_BAR[tone],
            )}
            style={{ width: `${Math.min(100, Math.max(0, fillPercent))}%` }}
          />
        </div>
      ) : null}
      {footer ? (
        <div className="mt-1.5 truncate text-[11px] leading-4 text-muted-foreground">
          {footer}
        </div>
      ) : null}
    </div>
  );
}
