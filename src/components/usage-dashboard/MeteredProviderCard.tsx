import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { useProviderUsageEvents } from "@/lib/query/usageDashboard";
import { useTranslation } from "react-i18next";

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
  return (
    <Card data-testid={`metered-provider-${usage.provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base">{usage.provider.name}</CardTitle>
          <div className="flex flex-wrap justify-end gap-2">
            <Badge variant="secondary">
              {t("usageDashboard.metered", { defaultValue: "Metered" })}
            </Badge>
          </div>
        </div>
        <div className="text-xs text-muted-foreground">{sourceText}</div>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        <div className="grid grid-cols-3 gap-3">
          <div>
            <div className="text-muted-foreground">
              {t("usageDashboard.tokens", { defaultValue: "Tokens" })}
            </div>
            <div className="font-semibold">{totalTokens.toLocaleString()}</div>
          </div>
          <div>
            <div className="text-muted-foreground">
              {t("usageDashboard.requests", { defaultValue: "Requests" })}
            </div>
            <div className="font-semibold">{usage.eventCount}</div>
          </div>
          <div>
            <div className="text-muted-foreground">USD</div>
            <div className="font-semibold">
              {usage.totalCostUsd ??
                t("usageDashboard.costUnavailableSummary", {
                  defaultValue: "Cost unavailable",
                })}
            </div>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          {providerCostBadges.map((source) => (
            <Badge key={source} variant="outline">
              {costSourceLabel(source)}
            </Badge>
          ))}
        </div>
        {events.error ? (
          <Alert
            variant="destructive"
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
          <div className="border-t pt-2">
            <div className="mb-1 text-xs font-medium text-muted-foreground">
              {t("usageDashboard.recentRequests", {
                defaultValue: "Recent requests",
              })}
            </div>
            {events.data.items.map((event) => (
              <div
                key={event.eventId}
                className="flex justify-between gap-2 text-xs"
              >
                <span className="truncate">{event.model}</span>
                <span>
                  {event.totalCostUsd == null
                    ? costSourceLabel(event.costSource)
                    : `$${event.totalCostUsd} · ${costSourceLabel(
                        event.costSource,
                      )}`}
                </span>
              </div>
            ))}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
