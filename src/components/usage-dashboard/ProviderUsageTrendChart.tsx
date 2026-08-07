import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import type {
  UsageTrendBucketView,
  UsageTrendGranularity,
} from "@/types/usageDashboard";
import { formatUsd } from "../tray-usage/trayUsagePresentation";
import { formatTokensCompact } from "./usagePresentation";
import type { MeteredCostStatus } from "./usageDashboardProjection";

interface ProviderUsageTrendChartProps {
  granularity: UsageTrendGranularity;
  buckets: UsageTrendBucketView[];
  totalTokens: number;
  /** Spend over the same range; subscription usage counts at list price. */
  totalCostUsd: string | null;
  costStatus: MeteredCostStatus;
  rangeLabel: string;
  rangeControls?: ReactNode;
}

type ChartPoint = UsageTrendBucketView & {
  label: string;
  tooltipLabel: string;
};

interface ProviderUsageTrendTooltipProps {
  active?: boolean;
  payload?: ReadonlyArray<{ payload?: ChartPoint }>;
}

export function ProviderUsageTrendTooltip({
  active,
  payload,
}: ProviderUsageTrendTooltipProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const point = payload?.[0]?.payload;
  if (!active || !point) return null;

  return (
    <div className="rounded-lg border border-border bg-popover px-3 py-2 shadow-pop">
      <p className="text-xs font-medium text-foreground">
        {point.tooltipLabel}
      </p>
      <p className="mt-1 text-xs text-muted-foreground metric">
        {formatTokensCompact(point.totalTokens)} Token ·{" "}
        {/* A bucket whose events were never priced says so rather than
            reading as a day that cost nothing. */}
        {point.totalCostUsd == null
          ? t("usageDashboard.costUnavailableSummary", {
              defaultValue: "Cost unavailable",
            })
          : formatUsd(point.totalCostUsd, locale)}
      </p>
    </div>
  );
}

export function ProviderUsageTrendChart({
  granularity,
  buckets,
  totalTokens,
  totalCostUsd,
  costStatus,
  rangeLabel,
  rangeControls,
}: ProviderUsageTrendChartProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const chartData = useMemo<ChartPoint[]>(
    () =>
      buckets.map((bucket) => {
        const date = new Date(bucket.startAt * 1_000);
        return {
          ...bucket,
          label:
            granularity === "hour"
              ? date.toLocaleTimeString(locale, {
                  hour: "2-digit",
                  minute: "2-digit",
                })
              : date.toLocaleDateString(locale, {
                  month: "numeric",
                  day: "numeric",
                }),
          tooltipLabel:
            granularity === "hour"
              ? date.toLocaleString(locale, {
                  month: "short",
                  day: "numeric",
                  hour: "2-digit",
                  minute: "2-digit",
                })
              : date.toLocaleDateString(locale, {
                  year: "numeric",
                  month: "short",
                  day: "numeric",
                }),
        };
      }),
    [buckets, granularity, locale],
  );
  const hasUsage = chartData.some((bucket) => bucket.totalTokens > 0);
  const peakTokens = chartData.reduce(
    (peak, bucket) => Math.max(peak, bucket.totalTokens),
    0,
  );
  const costStatusText =
    costStatus === "partial"
      ? t("usageDashboard.costPartial", {
          defaultValue: "Some usage could not be priced",
        })
      : costStatus === "estimated"
        ? t("usageDashboard.costEstimatedSummary", {
            defaultValue: "Includes estimated cost",
          })
        : costStatus === "unavailable"
          ? t("usageDashboard.costUnavailableSummary", {
              defaultValue: "Cost unavailable",
            })
          : null;

  return (
    <section aria-labelledby="usage-trend-heading">
      <Card className="overflow-hidden px-5 pb-3 pt-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 id="usage-trend-heading" className="text-sm font-semibold">
              {t("usageDashboard.usageTrend", {
                defaultValue: "Usage trend",
              })}
            </h2>
            <p className="mt-1 text-[11px] text-muted-foreground">
              <span>
                {granularity === "hour"
                  ? t("usageDashboard.hourlyBuckets", {
                      defaultValue: "Hourly",
                    })
                  : t("usageDashboard.dailyBuckets", {
                      defaultValue: "Daily",
                    })}
              </span>
              <span aria-hidden="true"> · </span>
              <span data-testid="usage-trend-range">{rangeLabel}</span>
            </p>
          </div>
          {/* Spend leads: it is the figure the range is being read for. The
              tokens that produced it stay underneath, where the peak gives the
              chart's tallest point a number. */}
          <div className="text-right">
            <div className="flex flex-wrap items-center justify-end gap-2">
              <div className="text-lg font-semibold metric">
                {totalCostUsd == null
                  ? t("usageDashboard.costUnavailableSummary", {
                      defaultValue: "Cost unavailable",
                    })
                  : formatUsd(totalCostUsd, locale)}
              </div>
              {costStatusText ? (
                <Badge variant="outline">{costStatusText}</Badge>
              ) : null}
            </div>
            <p
              className="mt-0.5 text-[11px] text-muted-foreground"
              title={totalTokens.toLocaleString(locale)}
            >
              {t("usageDashboard.peakTokens", {
                value: `${formatTokensCompact(peakTokens)} Token`,
                defaultValue: "Peak {{value}}",
              })}
              {" · "}
              {t("usageDashboard.totalTokensNamed", {
                defaultValue: "Tokens",
              })}{" "}
              {formatTokensCompact(totalTokens)}
            </p>
          </div>
        </div>

        {rangeControls ? (
          <div className="mt-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/60 bg-muted/20 px-3 py-2.5">
            <p className="text-xs font-medium text-foreground">
              {t("usage.timeRange", { defaultValue: "Time range" })}
            </p>
            {rangeControls}
          </div>
        ) : null}

        {hasUsage ? (
          <div
            className="mt-4 h-72 w-full"
            role="img"
            aria-label={t("usageDashboard.trendChartLabel", {
              defaultValue: "Token usage by {{granularity}}",
              granularity:
                granularity === "hour"
                  ? t("usageDashboard.hour", { defaultValue: "hour" })
                  : t("usageDashboard.day", { defaultValue: "day" }),
            })}
          >
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart
                data={chartData}
                margin={{ top: 8, right: 10, left: 4, bottom: 0 }}
              >
                <CartesianGrid
                  vertical={false}
                  stroke="hsl(var(--border) / 0.55)"
                  strokeDasharray="3 4"
                />
                <XAxis
                  dataKey="label"
                  axisLine={false}
                  tickLine={false}
                  interval="preserveStartEnd"
                  minTickGap={28}
                  tick={{
                    fill: "hsl(var(--muted-foreground))",
                    fontSize: 10,
                  }}
                  dy={7}
                />
                <YAxis hide domain={[0, "dataMax"]} />
                <Tooltip
                  content={ProviderUsageTrendTooltip}
                  cursor={{ stroke: "hsl(var(--border))" }}
                />
                <Area
                  type="monotone"
                  dataKey="totalTokens"
                  stroke="hsl(var(--primary))"
                  strokeWidth={2.5}
                  fill="hsl(var(--primary) / 0.14)"
                  dot={false}
                  activeDot={{ r: 4, strokeWidth: 2 }}
                  isAnimationActive={false}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        ) : (
          <div className="mt-4 grid h-48 place-items-center rounded-lg bg-muted/20 text-sm text-muted-foreground dark:bg-muted/10">
            {t("usageDashboard.trendEmpty", {
              defaultValue: "No usage recorded in this range.",
            })}
          </div>
        )}
      </Card>
    </section>
  );
}
