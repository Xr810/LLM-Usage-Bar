import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { useProviderUsageEvents } from "@/lib/query/usageDashboard";
import { useTranslation } from "react-i18next";
import {
  dashboardProviderIcon,
  formatTokensCompact,
} from "./usagePresentation";

export function MeteredProviderCard({
  usage,
  startAt,
  endAt,
}: {
  usage: ProviderUsageView;
  startAt: number;
  endAt: number;
}) {
  const { t } = useTranslation();
  const events = useProviderUsageEvents(
    usage.provider.id,
    startAt,
    endAt,
    1,
    5,
  );
  const totalTokens =
    usage.inputTokens +
    usage.outputTokens +
    usage.cacheReadTokens +
    usage.cacheCreationTokens;
  const sourceText = usage.provider.tokenSources
    .map((source) =>
      source === "proxy"
        ? t("usageDashboard.sourceProxy", { defaultValue: "Proxy" })
        : t("usageDashboard.sourceSession", { defaultValue: "Session log" }),
    )
    .join(" + ");
  const costSourceLabel = (source: string) => {
    if (source === "upstream") {
      return t("usageDashboard.costUpstream", { defaultValue: "Upstream" });
    }
    if (source === "estimated") {
      return t("usageDashboard.costEstimated", { defaultValue: "Estimated" });
    }
    return t("usageDashboard.costUnavailable", {
      defaultValue: "Unavailable",
    });
  };
  const providerCostBadges = [
    usage.costSourceCounts.upstream > 0 ? "upstream" : null,
    usage.costSourceCounts.estimated > 0 ? "estimated" : null,
    usage.costSourceCounts.unavailable > 0 ? "unavailable" : null,
  ].filter((source): source is string => source != null);
  const { icon, iconColor } = dashboardProviderIcon(usage.provider);

  const stats = [
    {
      label: t("usageDashboard.tokens", { defaultValue: "Tokens" }),
      value: formatTokensCompact(totalTokens),
      title: totalTokens.toLocaleString(),
    },
    {
      label: t("usageDashboard.requests", { defaultValue: "Requests" }),
      value: usage.eventCount.toLocaleString(),
      title: undefined,
    },
    {
      label: "USD",
      // No events in this range means nothing was spent. Only an account that
      // has usage the app could not price has a cost it cannot report.
      value:
        usage.totalCostUsd != null
          ? `$${usage.totalCostUsd}`
          : usage.eventCount === 0
            ? "$0"
            : t("usageDashboard.costUnavailableSummary", {
                defaultValue: "Cost unavailable",
              }),
      title: undefined,
    },
  ];

  return (
    <Card
      data-testid={`metered-provider-${usage.provider.id}`}
      className="overflow-hidden"
    >
      <div className="flex items-center justify-between gap-3 px-5 pt-4">
        <div className="flex min-w-0 items-center gap-3">
          <ProviderIcon
            icon={icon}
            color={iconColor}
            name={usage.provider.name}
            size={34}
            className="shrink-0 rounded-[10px] border border-border/50"
          />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="truncate text-[15px] font-semibold tracking-tight">
                {usage.provider.name}
              </h3>
              <Badge variant="secondary" className="shrink-0">
                {t("usageDashboard.metered", { defaultValue: "Metered" })}
              </Badge>
            </div>
            <p className="mt-0.5 truncate text-xs text-muted-foreground">
              {sourceText}
            </p>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap justify-end gap-1.5">
          {providerCostBadges.map((source) => (
            <Badge key={source} variant="outline">
              {costSourceLabel(source)}
            </Badge>
          ))}
        </div>
      </div>

      <div className="px-5 pt-4 pb-4">
        <dl className="grid grid-cols-3 gap-2 rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15">
          {stats.map((stat) => (
            <div key={stat.label} className="min-w-0">
              <dt className="truncate text-[11px] text-muted-foreground">
                {stat.label}
              </dt>
              <dd
                className="mt-0.5 truncate text-sm font-semibold metric"
                title={stat.title}
              >
                {stat.value}
              </dd>
            </div>
          ))}
        </dl>

        {events.error ? (
          <Alert
            variant="destructive"
            className="mt-3"
            aria-label={
              events.error instanceof Error
                ? events.error.message
                : String(events.error)
            }
          >
            <AlertDescription>
              {events.error instanceof Error
                ? events.error.message
                : String(events.error)}
            </AlertDescription>
          </Alert>
        ) : null}
        {events.data?.items.length ? (
          <div className="mt-3 border-t border-border/60 pt-3">
            <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              {t("usageDashboard.recentRequests", {
                defaultValue: "Recent requests",
              })}
            </div>
            <ul className="space-y-1">
              {events.data.items.map((event) => (
                <li
                  key={event.eventId}
                  className="flex items-baseline justify-between gap-3 text-xs"
                >
                  <span className="truncate font-mono text-[11px] text-foreground/80">
                    {event.model}
                  </span>
                  <span className="shrink-0 metric text-muted-foreground">
                    {event.totalCostUsd == null
                      ? costSourceLabel(event.costSource)
                      : `$${event.totalCostUsd} · ${costSourceLabel(
                          event.costSource,
                        )}`}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    </Card>
  );
}
