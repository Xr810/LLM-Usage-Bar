import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Bar, BarChart, ResponsiveContainer, Tooltip } from "recharts";
import type { TrayProviderRecentUsageView } from "@/types/trayUsage";
import { formatTokenCount, formatUsd } from "./trayUsagePresentation";

export function TrayProviderRecentUsage({
  usage,
  locale,
}: {
  usage: TrayProviderRecentUsageView;
  locale: string;
}) {
  const { t } = useTranslation();
  const data = useMemo(
    () =>
      usage.trendBuckets.map((bucket) => ({
        ...bucket,
        label: new Date(bucket.startAt * 1_000).toLocaleDateString(locale, {
          month: "short",
          day: "numeric",
        }),
      })),
    [locale, usage.trendBuckets],
  );
  const hasUsage = data.some((bucket) => bucket.totalTokens > 0);
  const recentTokens =
    [...data].reverse().find((bucket) => bucket.totalTokens > 0)?.totalTokens ??
    0;
  const cost = usage.costQuality === "unavailable" ? null : usage.totalCostUsd;
  const todayCost =
    usage.costQuality === "unavailable" ? null : usage.todayCostUsd;

  const tooltip = ({ active, payload }: any) => {
    const point = payload?.[0]?.payload as
      { label: string; totalTokens: number; eventCount: number } | undefined;
    if (!active || !point) return null;
    return (
      <div className="rounded-md border border-border bg-popover px-2 py-1.5 shadow-pop">
        <p className="text-[10px] font-medium text-foreground">{point.label}</p>
        <p className="mt-0.5 text-[10px] tabular-nums text-muted-foreground">
          {formatTokenCount(point.totalTokens, locale)} · {point.eventCount}
        </p>
      </div>
    );
  };

  return (
    <div className="space-y-2 border-t border-border/50 pt-2.5">
      <dl className="grid grid-cols-2 gap-x-5 gap-y-2 px-0.5">
        <div className="min-w-0">
          <dt className="truncate text-[10px] font-medium text-muted-foreground">
            {t("trayUsage.today", { defaultValue: "Today" })}
          </dt>
          <dd className="mt-0.5 truncate text-[15px] font-semibold tabular-nums">
            {formatUsd(todayCost, locale)}
          </dd>
        </div>
        <div className="min-w-0">
          <dt className="truncate text-[10px] font-medium text-muted-foreground">
            {t("trayUsage.rolling30DayCost", {
              defaultValue: "Last 30 days cost",
            })}
          </dt>
          <dd className="mt-0.5 truncate text-[15px] font-semibold tabular-nums">
            {formatUsd(cost, locale)}
          </dd>
        </div>
        <div className="min-w-0">
          <dt className="truncate text-[10px] font-medium text-muted-foreground">
            {t("trayUsage.rolling30DayTokens", {
              defaultValue: "Last 30 days tokens",
            })}
          </dt>
          <dd className="mt-0.5 truncate text-[15px] font-semibold tabular-nums">
            {formatTokenCount(usage.totalTokens, locale)}
          </dd>
        </div>
        <div className="min-w-0">
          <dt className="truncate text-[10px] font-medium text-muted-foreground">
            {t("trayUsage.recentTokens", {
              defaultValue: "Recent token usage",
            })}
          </dt>
          <dd className="mt-0.5 truncate text-[15px] font-semibold tabular-nums">
            {formatTokenCount(recentTokens, locale)}
          </dd>
        </div>
      </dl>

      <div
        className="h-[74px] w-full overflow-hidden rounded-md bg-muted/10 px-1 pt-1"
        role="img"
        aria-label={t("trayUsage.providerTrendChart", {
          defaultValue: "Token usage for the last 30 days",
        })}
      >
        {hasUsage ? (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart
              data={data}
              margin={{ top: 3, right: 0, left: 0, bottom: 0 }}
            >
              <Tooltip
                content={tooltip}
                cursor={{ fill: "hsl(var(--muted) / 0.3)" }}
              />
              <Bar
                dataKey="totalTokens"
                fill="#D89232"
                fillOpacity={0.92}
                radius={[2, 2, 0, 0]}
                maxBarSize={9}
                isAnimationActive={false}
              />
            </BarChart>
          </ResponsiveContainer>
        ) : (
          <div className="grid h-full place-items-center text-[10px] text-muted-foreground">
            {t("trayUsage.noRecentUsage", {
              defaultValue: "No recent usage",
            })}
          </div>
        )}
      </div>

      <div className="border-t border-border/50 px-0.5 pt-2 text-[10px] leading-snug text-muted-foreground">
        <p>
          {t("trayUsage.mostUsedModel", {
            model:
              usage.mostUsedModel ??
              t("trayUsage.unknown", { defaultValue: "Unknown" }),
            defaultValue: "Most used model: {{model}}",
          })}
        </p>
      </div>
    </div>
  );
}
