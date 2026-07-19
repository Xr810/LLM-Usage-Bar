import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
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
import type { MeteredCostStatus } from "./usageDashboardProjection";

interface ProviderUsageTrendChartProps {
  granularity: UsageTrendGranularity;
  buckets: UsageTrendBucketView[];
  totalTokens: number;
  requestCount: number;
  totalCostUsd: string | null;
  costStatus: MeteredCostStatus;
}

export function ProviderUsageTrendChart({
  granularity,
  buckets,
  totalTokens,
  requestCount,
  totalCostUsd,
  costStatus,
}: ProviderUsageTrendChartProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
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
          cost:
            bucket.totalCostUsd == null
              ? null
              : Number.parseFloat(bucket.totalCostUsd),
        };
      }),
    [buckets, granularity, locale],
  );
  const hasUsage = chartData.some((bucket) => bucket.totalTokens > 0);
  const tokenBreakdown = buckets.reduce(
    (total, bucket) => ({
      input: total.input + bucket.inputTokens,
      output: total.output + bucket.outputTokens,
      cacheRead: total.cacheRead + bucket.cacheReadTokens,
      cacheCreation: total.cacheCreation + bucket.cacheCreationTokens,
    }),
    { input: 0, output: 0, cacheRead: 0, cacheCreation: 0 },
  );
  const cacheHitDenominator = tokenBreakdown.input + tokenBreakdown.cacheRead;
  const cacheHitRate = cacheHitDenominator
    ? (tokenBreakdown.cacheRead / cacheHitDenominator) * 100
    : null;

  const tooltip = ({ active, payload }: any) => {
    const point = payload?.[0]?.payload as
      (UsageTrendBucketView & { tooltipLabel: string }) | undefined;
    if (!active || !point) return null;
    return (
      <div className="rounded-lg border border-border bg-popover px-3 py-2 shadow-pop">
        <p className="text-xs font-medium text-foreground">
          {point.tooltipLabel}
        </p>
        <div className="mt-1.5 space-y-0.5 text-xs metric text-muted-foreground">
          <p>
            {t("usageDashboard.inputTokens", { defaultValue: "Input" })}:{" "}
            {point.inputTokens.toLocaleString(locale)}
          </p>
          <p>
            {t("usageDashboard.outputTokens", { defaultValue: "Output" })}:{" "}
            {point.outputTokens.toLocaleString(locale)}
          </p>
          <p>
            {t("usageDashboard.cacheCreationTokens", {
              defaultValue: "Cache creation",
            })}
            : {point.cacheCreationTokens.toLocaleString(locale)}
          </p>
          <p>
            {t("usageDashboard.cacheReadTokens", {
              defaultValue: "Cache read",
            })}
            : {point.cacheReadTokens.toLocaleString(locale)}
          </p>
          <p>
            {t("usage.totalCost", { defaultValue: "Total cost" })}:{" "}
            {point.totalCostUsd == null ? "—" : `$${point.totalCostUsd}`}
          </p>
        </div>
      </div>
    );
  };

  return (
    <section aria-labelledby="usage-trend-heading">
      <Card className="overflow-hidden px-5 pt-4 pb-3">
        <div className="flex flex-wrap items-start justify-between gap-5">
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
            <div
              className="mt-3 text-3xl font-semibold tracking-tight metric"
              title={totalTokens.toLocaleString(locale)}
            >
              {totalTokens.toLocaleString(locale)}
            </div>
            <div className="mt-0.5 text-[11px] text-muted-foreground">
              {t("usageDashboard.realTokenUsage", {
                defaultValue: "Total token usage",
              })}
            </div>
          </div>
          <div className="flex shrink-0 items-start gap-6 rounded-xl border border-border/60 bg-muted/15 px-4 py-3 text-right">
            <div>
              <div className="text-lg font-semibold metric">
                {requestCount.toLocaleString(locale)}
              </div>
              <div className="text-[11px] text-muted-foreground">
                {t("usageDashboard.requests", { defaultValue: "Requests" })}
              </div>
            </div>
            <div>
              <div className="text-lg font-semibold metric">
                {totalCostUsd == null ? "—" : `$${totalCostUsd}`}
              </div>
              <div className="text-[11px] text-muted-foreground">
                {costStatus === "estimated"
                  ? t("usageDashboard.estimatedCost", {
                      defaultValue: "Estimated cost",
                    })
                  : costStatus === "partial"
                    ? t("usageDashboard.partialEstimatedCost", {
                        defaultValue: "Partial estimated cost",
                      })
                    : costStatus === "unavailable"
                      ? t("usageDashboard.costUnavailableSummary", {
                          defaultValue: "Cost unavailable",
                        })
                      : t("usage.totalCost", {
                          defaultValue: "Total cost",
                        })}
              </div>
            </div>
          </div>
        </div>

        <dl className="mt-4 grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
          {[
            [
              t("usageDashboard.inputTokens", { defaultValue: "Input" }),
              tokenBreakdown.input,
            ],
            [
              t("usageDashboard.outputTokens", { defaultValue: "Output" }),
              tokenBreakdown.output,
            ],
            [
              t("usageDashboard.cacheCreationTokens", {
                defaultValue: "Cache creation",
              }),
              tokenBreakdown.cacheCreation,
            ],
            [
              t("usageDashboard.cacheReadTokens", {
                defaultValue: "Cache read",
              }),
              tokenBreakdown.cacheRead,
            ],
          ].map(([label, value]) => (
            <div
              key={String(label)}
              className="rounded-lg border border-border/50 bg-muted/10 px-3 py-2.5"
            >
              <dt className="truncate text-[11px] text-muted-foreground">
                {label}
              </dt>
              <dd
                className="mt-1 text-base font-semibold metric"
                title={Number(value).toLocaleString(locale)}
              >
                {formatTokensCompact(Number(value))}
              </dd>
            </div>
          ))}
          <div className="rounded-lg border border-border/50 bg-muted/10 px-3 py-2.5">
            <dt className="text-[11px] text-muted-foreground">
              {t("usageDashboard.cacheHitRate", {
                defaultValue: "Cache hit rate",
              })}
            </dt>
            <dd className="mt-1 text-base font-semibold metric">
              {cacheHitRate == null ? "—" : `${cacheHitRate.toFixed(1)}%`}
            </dd>
          </div>
        </dl>

        {hasUsage ? (
          <div
            className="mt-5 h-72 w-full"
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
              <ComposedChart
                data={chartData}
                margin={{ top: 8, right: 12, left: 2, bottom: 0 }}
              >
                <CartesianGrid
                  vertical={false}
                  stroke="hsl(var(--border) / 0.45)"
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
                <YAxis yAxisId="tokens" hide domain={[0, "dataMax"]} />
                <YAxis
                  yAxisId="cost"
                  orientation="right"
                  hide
                  domain={[0, "dataMax"]}
                />
                <Tooltip
                  content={tooltip}
                  cursor={{ fill: "hsl(var(--muted) / 0.35)" }}
                />
                <Legend wrapperStyle={{ fontSize: 10, paddingTop: 8 }} />
                <Bar
                  yAxisId="tokens"
                  dataKey="inputTokens"
                  name={t("usageDashboard.inputTokens", {
                    defaultValue: "Input",
                  })}
                  stackId="tokens"
                  fill="#3B82F6"
                  maxBarSize={26}
                  isAnimationActive={false}
                />
                <Bar
                  yAxisId="tokens"
                  dataKey="outputTokens"
                  name={t("usageDashboard.outputTokens", {
                    defaultValue: "Output",
                  })}
                  stackId="tokens"
                  fill="#22C55E"
                  maxBarSize={26}
                  isAnimationActive={false}
                />
                <Bar
                  yAxisId="tokens"
                  dataKey="cacheCreationTokens"
                  name={t("usageDashboard.cacheCreationTokens", {
                    defaultValue: "Cache creation",
                  })}
                  stackId="tokens"
                  fill="#F97316"
                  maxBarSize={26}
                  isAnimationActive={false}
                />
                <Bar
                  yAxisId="tokens"
                  dataKey="cacheReadTokens"
                  name={t("usageDashboard.cacheReadTokens", {
                    defaultValue: "Cache read",
                  })}
                  stackId="tokens"
                  fill="#A855F7"
                  radius={[3, 3, 0, 0]}
                  maxBarSize={26}
                  isAnimationActive={false}
                />
                <Line
                  yAxisId="cost"
                  type="monotone"
                  dataKey="cost"
                  name={t("usage.totalCost", { defaultValue: "Cost" })}
                  stroke="#F43F5E"
                  strokeWidth={2}
                  strokeDasharray="5 4"
                  dot={false}
                  connectNulls
                  isAnimationActive={false}
                />
              </ComposedChart>
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
