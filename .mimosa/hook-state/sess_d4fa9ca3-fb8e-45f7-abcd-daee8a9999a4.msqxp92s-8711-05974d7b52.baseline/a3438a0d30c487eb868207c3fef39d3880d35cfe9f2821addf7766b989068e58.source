import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Card } from "@/components/ui/card";
import { costText } from "./breakdownPresentation";
import { formatTokensCompact } from "./usagePresentation";

interface BreakdownSummaryProps {
  totalTokens: number;
  totalEventCount: number;
  totalCostUsd: string | null;
  /** Human range text, e.g. "30 days" or "Mar 1 – Mar 31". */
  rangeLabel: string;
  rangeControls?: ReactNode;
  /** Right-hand caption, e.g. "12 models" or "4 agents". */
  countLabel?: string;
}

/**
 * Range totals for a breakdown page, carrying the range picker so the
 * "By model" and "By agent" tabs stay switchable without losing the range.
 */
export function BreakdownSummary({
  totalTokens,
  totalEventCount,
  totalCostUsd,
  rangeLabel,
  rangeControls,
  countLabel,
}: BreakdownSummaryProps) {
  const { t } = useTranslation();

  const stats = [
    {
      key: "tokens",
      label: t("usageDashboard.tokens", { defaultValue: "Tokens" }),
      value: formatTokensCompact(totalTokens),
      title: totalTokens.toLocaleString(),
      testId: "breakdown-total-tokens",
    },
    {
      key: "requests",
      label: t("usageDashboard.requests", { defaultValue: "Requests" }),
      value: totalEventCount.toLocaleString(),
      title: undefined,
      testId: "breakdown-total-requests",
    },
    {
      key: "usd",
      label: "USD",
      value: costText(totalCostUsd, t),
      title: undefined,
      testId: "breakdown-total-cost",
    },
  ];

  return (
    <Card className="overflow-hidden">
      <div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3 px-5 py-4">
        <div className="flex min-w-0 items-end gap-6">
          {stats.map((stat) => (
            <div key={stat.key} className="min-w-0">
              <div className="text-[11px] text-muted-foreground">
                {stat.label}
              </div>
              <div
                className="mt-0.5 text-lg font-semibold metric"
                data-testid={stat.testId}
                title={stat.title}
              >
                {stat.value}
              </div>
            </div>
          ))}
        </div>
        <div className="flex flex-wrap items-center justify-end gap-3">
          <div className="text-right">
            <div
              className="text-xs font-medium"
              data-testid="breakdown-range-label"
            >
              {rangeLabel}
            </div>
            {countLabel ? (
              <div className="mt-0.5 text-[11px] text-muted-foreground">
                {countLabel}
              </div>
            ) : null}
          </div>
          {rangeControls}
        </div>
      </div>
    </Card>
  );
}
