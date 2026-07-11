import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { useUsageEvents } from "@/lib/query/usageDashboard";
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
  const events = useUsageEvents(usage.provider.id, startAt, endAt, 1, 5);
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
  return (
    <Card data-testid={`metered-provider-${usage.provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base">{usage.provider.name}</CardTitle>
          <Badge variant="secondary">
            {t("usageDashboard.metered", { defaultValue: "Metered" })}
          </Badge>
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
            <div className="font-semibold">{usage.totalCostUsd ?? "—"}</div>
          </div>
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
                    ? event.costSource
                    : `$${event.totalCostUsd}`}
                </span>
              </div>
            ))}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
