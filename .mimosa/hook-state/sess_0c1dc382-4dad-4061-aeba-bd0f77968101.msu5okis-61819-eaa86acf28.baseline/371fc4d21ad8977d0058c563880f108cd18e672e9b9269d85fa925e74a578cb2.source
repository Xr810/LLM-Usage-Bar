import { useId, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatSharePercent } from "./breakdownPresentation";
import { formatTokensCompact } from "./usagePresentation";

interface UsageBreakdownRowProps {
  /** Primary label — a plan name, a model id, an Agent name. */
  title: ReactNode;
  /** Secondary line under the title: accounts, last-used time, source. */
  subtitle?: ReactNode;
  /** Rendered before the title: a brand icon or a rank marker. */
  leading?: ReactNode;
  badges?: ReactNode;
  /** Share of the range total, 0–100. Drives the bar behind the row. */
  share: number;
  tokens: number;
  requests: number;
  /** Already formatted — `$1.25` or the "unavailable" text. */
  costText: string;
  costMuted?: boolean;
  /** Present only for rows that drill down; makes the row a disclosure. */
  children?: ReactNode;
  defaultExpanded?: boolean;
  /** `nested` rows sit inside an expanded parent and read one level quieter. */
  variant?: "primary" | "nested";
  "data-testid"?: string;
}

/**
 * One row of a ranked breakdown.
 *
 * The share is drawn as a tinted fill behind the row rather than as a separate
 * bar column, so a long list reads as a bar chart at a glance while every row
 * still carries its exact numbers.
 */
export function UsageBreakdownRow({
  title,
  subtitle,
  leading,
  badges,
  share,
  tokens,
  requests,
  costText,
  costMuted = false,
  children,
  defaultExpanded = false,
  variant = "primary",
  "data-testid": testId,
}: UsageBreakdownRowProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const panelId = useId();
  const expandable = Boolean(children);
  const nested = variant === "nested";

  const content = (
    <>
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-y-0 left-0 rounded-r-[3px] transition-[width] duration-300",
          nested ? "bg-primary/[0.07]" : "bg-primary/10",
        )}
        style={{ width: `${Math.max(share, share > 0 ? 0.6 : 0)}%` }}
      />
      <span className="relative flex min-w-0 flex-1 items-center gap-2.5">
        {expandable ? (
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90",
            )}
            aria-hidden="true"
          />
        ) : null}
        {leading}
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span
              className={cn(
                "truncate",
                nested
                  ? "font-mono text-[12px] text-foreground/85"
                  : "text-sm font-semibold tracking-tight",
              )}
            >
              {title}
            </span>
            {badges}
          </span>
          {subtitle ? (
            <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">
              {subtitle}
            </span>
          ) : null}
        </span>
      </span>

      <span className="relative flex shrink-0 items-center gap-3 sm:gap-5">
        <span className="hidden w-11 text-right text-[11px] tabular-nums text-muted-foreground sm:inline">
          {formatSharePercent(share)}
        </span>
        <span
          className="w-14 text-right text-[13px] font-semibold metric"
          title={`${tokens.toLocaleString()} tokens`}
        >
          {formatTokensCompact(tokens)}
        </span>
        <span className="hidden w-12 text-right text-[12px] metric text-muted-foreground min-[560px]:inline">
          {requests.toLocaleString()}
        </span>
        <span
          className={cn(
            "w-[5.5rem] truncate text-right text-[12px] metric",
            costMuted ? "text-muted-foreground" : "text-foreground/90",
          )}
          title={costText}
        >
          {costText}
        </span>
      </span>
    </>
  );

  const rowClass = cn(
    "relative flex w-full items-center gap-3 overflow-hidden px-3 text-left",
    nested ? "py-2" : "py-2.5",
    expandable && "transition-colors hover:bg-muted/40",
  );

  return (
    <div data-testid={testId}>
      {expandable ? (
        <button
          type="button"
          className={cn(
            rowClass,
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/40",
          )}
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={() => setExpanded((open) => !open)}
        >
          {content}
        </button>
      ) : (
        <div className={rowClass}>{content}</div>
      )}
      {expandable && expanded ? (
        <div
          id={panelId}
          className="border-t border-border/40 bg-muted/15 pl-6"
        >
          {children}
        </div>
      ) : null}
    </div>
  );
}

/** Column captions for a breakdown list, aligned with `UsageBreakdownRow`. */
export function UsageBreakdownHeader({
  label,
  tokensLabel,
  requestsLabel,
  shareLabel,
}: {
  label: string;
  tokensLabel: string;
  requestsLabel: string;
  shareLabel: string;
}) {
  return (
    <div className="flex items-center gap-3 border-b border-border/60 px-3 pb-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
      <span className="min-w-0 flex-1">{label}</span>
      <span className="flex shrink-0 items-center gap-3 sm:gap-5">
        <span className="hidden w-11 text-right sm:inline">{shareLabel}</span>
        <span className="w-14 text-right">{tokensLabel}</span>
        <span className="hidden w-12 text-right min-[560px]:inline">
          {requestsLabel}
        </span>
        <span className="w-[5.5rem] text-right">USD</span>
      </span>
    </div>
  );
}
