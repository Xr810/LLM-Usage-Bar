import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import { Bar, BarChart, ResponsiveContainer, Tooltip } from "recharts";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
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
  const [open, setOpen] = useState(false);
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
      | { label: string; totalTokens: number; totalCostUsd: string | null }
      | undefined;
    if (!active || !point) return null;
    // Tokens answer "how much did I use", spend answers "what did it cost" —
    // the request count answered neither.
    const cost =
      usage.costQuality === "unavailable" ? null : point.totalCostUsd;
    return (
      <div className="rounded-md border border-border bg-popover px-2 py-1.5 shadow-pop">
        <p className="text-[10px] font-medium text-foreground">{point.label}</p>
        <p className="mt-0.5 text-[10px] tabular-nums text-muted-foreground">
          {formatTokenCount(point.totalTokens, locale)} ·{" "}
          {formatUsd(cost, locale)}
        </p>
      </div>
    );
  };

  return (
    // The popover is 380×520. Spend rides along the always-visible summary row
    // because it is the number worth glancing at; the token breakdown, the
    // sparkline and the model line move behind a disclosure so several accounts
    // fit on screen at once instead of one filling more than the whole popover.
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="border-t border-border/50 pt-2"
    >
      <CollapsibleTrigger
        aria-label={t("trayUsage.toggleUsageDetails", {
          defaultValue: "Usage details",
        })}
        className="flex w-full items-center justify-between gap-2 rounded-md px-0.5 py-1 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="flex min-w-0 items-baseline gap-1.5 text-[11px] text-muted-foreground">
          <span className="shrink-0">
            {t("trayUsage.today", { defaultValue: "Today" })}
          </span>
          <span className="truncate text-[13px] font-semibold tabular-nums text-foreground">
            {formatUsd(todayCost, locale)}
          </span>
          <span aria-hidden="true" className="shrink-0">
            ·
          </span>
          <span className="shrink-0">
            {t("trayUsage.rolling30DayCost", {
              defaultValue: "Last 30 days cost",
            })}
          </span>
          <span className="truncate text-[13px] font-semibold tabular-nums text-foreground">
            {formatUsd(cost, locale)}
          </span>
        </span>
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-180",
          )}
        />
      </CollapsibleTrigger>

      <CollapsibleContent className="space-y-2 pt-2">
        <dl className="grid grid-cols-2 gap-x-5 gap-y-2 px-0.5">
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
      </CollapsibleContent>
    </Collapsible>
  );
}
