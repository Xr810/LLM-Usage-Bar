import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Bar,
  BarChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Card } from "@/components/ui/card";
import type {
  UsageTrendBucketView,
  UsageTrendGranularity,
} from "@/types/usageDashboard";
import { formatTokensCompact } from "./usagePresentation";

interface ProviderUsageTrendChartProps {
  granularity: UsageTrendGranularity;
  buckets: UsageTrendBucketView[];
}

export function ProviderUsageTrendChart({
  granularity,
  buckets,
}: ProviderUsageTrendChartProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const totalTokens = buckets.reduce(
    (total, bucket) => total + bucket.totalTokens,
    0,
  );
  const chartData = useMemo(
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

  const tooltip = ({ active, payload }: any) => {
    const point = payload?.[0]?.payload as
      (UsageTrendBucketView & { tooltipLabel: string }) | undefined;
    if (!active || !point) return null;
    return (
      <div className="rounded-lg border border-border bg-popover px-3 py-2 shadow-pop">
        <p className="text-xs font-medium text-foreground">
          {point.tooltipLabel}
        </p>
        <p className="mt-1 text-xs metric text-muted-foreground">
          {t("usageDashboard.trendTooltip", {
            defaultValue: "{{tokens}} tokens · {{requests}} requests",
            tokens: point.totalTokens.toLocaleString(locale),
            requests: point.eventCount.toLocaleString(locale),
          })}
        </p>
      </div>
    );
  };

  return (
    <section aria-labelledby="usage-trend-heading">
      <Card className="overflow-hidden px-5 pt-4 pb-3">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2
              id="usage-trend-heading"
              className="text-sm font-semibold tracking-tight"
            >
              {t("usageDashboard.usageTrend", {
                defaultValue: "Usage trend",
              })}
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {granularity === "hour"
                ? t("usageDashboard.hourlyBuckets", {
                    defaultValue: "Hourly",
                  })
                : t("usageDashboard.dailyBuckets", {
                    defaultValue: "Daily",
                  })}
            </p>
          </div>
          <div className="text-right">
            <div
              className="text-lg font-semibold metric"
              title={totalTokens.toLocaleString(locale)}
            >
              {formatTokensCompact(totalTokens)}
            </div>
            <div className="text-[11px] text-muted-foreground">
              {t("usageDashboard.totalTokens", { defaultValue: "Total" })}
            </div>
          </div>
        </div>

        {hasUsage ? (
          <div
            className="mt-3 h-52 w-full"
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
              <BarChart
                data={chartData}
                margin={{ top: 8, right: 2, left: 2, bottom: 0 }}
              >
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
                  content={tooltip}
                  cursor={{ fill: "hsl(var(--muted) / 0.35)" }}
                />
                <Bar
                  dataKey="totalTokens"
                  fill="hsl(var(--primary))"
                  fillOpacity={0.82}
                  radius={[4, 4, 1, 1]}
                  maxBarSize={30}
                />
              </BarChart>
            </ResponsiveContainer>
          </div>
        ) : (
          <div className="mt-3 grid h-40 place-items-center rounded-lg bg-muted/20 text-sm text-muted-foreground dark:bg-muted/10">
            {t("usageDashboard.trendEmpty", {
              defaultValue: "No usage recorded in this range.",
            })}
          </div>
        )}
      </Card>
    </section>
  );
}
