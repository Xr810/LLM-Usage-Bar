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
  /** Removes the inset card treatment when the meter already sits in a sidebar card. */
  flat?: boolean;
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
  flat = false,
}: QuotaMeterProps) {
  return (
    <div
      className={cn(
        "py-2.5",
        flat
          ? "border-0 px-0"
          : "rounded-lg border border-border/50 bg-muted/25 px-3 dark:bg-muted/15",
      )}
    >
      {/* Every meter keeps the same three rows — label, track, caption — so two
          of them side by side line up even when one window has no data. The
          figure slot is `shrink-0`, so an unavailable window leaves it empty and
          explains itself in the caption instead; putting the sentence there
          starved the label down to "5 …". */}
      <div className="flex items-baseline justify-between gap-2">
        <span className="truncate text-xs text-muted-foreground">{label}</span>
        {fillPercent != null ? (
          <span className="shrink-0 text-sm font-semibold tracking-tight metric">
            {valueText}
          </span>
        ) : null}
      </div>
      <div
        role={fillPercent != null ? "progressbar" : undefined}
        aria-label={fillPercent != null ? meterLabel : undefined}
        aria-valuemin={fillPercent != null ? 0 : undefined}
        aria-valuemax={fillPercent != null ? 100 : undefined}
        aria-valuenow={
          fillPercent != null ? Math.round(fillPercent) : undefined
        }
        className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted"
      >
        {fillPercent != null ? (
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-500 ease-out",
              TONE_BAR[tone],
            )}
            style={{ width: `${Math.min(100, Math.max(0, fillPercent))}%` }}
          />
        ) : null}
      </div>
      {/* The caption carries the reset countdown, or the reason this window has
          nothing to show. It wraps rather than truncating: the sentence is the
          only thing explaining an empty track. */}
      {(footer ?? (fillPercent == null ? valueText : null)) ? (
        <div className="mt-1.5 text-[11px] leading-4 text-muted-foreground">
          {footer ?? valueText}
        </div>
      ) : null}
    </div>
  );
}
