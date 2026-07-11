import { Badge } from "@/components/ui/badge";
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
  return (
    <Card data-testid={`metered-provider-${usage.provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base">{usage.provider.name}</CardTitle>
          <Badge variant="secondary">
            {t("usageDashboard.metered", { defaultValue: "Metered" })}
          </Badge>
        </div>
        <div className="text-xs text-muted-foreground">
          {t("usageDashboard.sourceProxy", { defaultValue: "Proxy" })}
        </div>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        <div className="grid grid-cols-3 gap-3">
          <div>
            <div className="text-muted-foreground">Tokens</div>
            <div className="font-semibold">{totalTokens.toLocaleString()}</div>
          </div>
          <div>
            <div className="text-muted-foreground">Requests</div>
            <div className="font-semibold">{usage.eventCount}</div>
          </div>
          <div>
            <div className="text-muted-foreground">USD</div>
            <div className="font-semibold">{usage.totalCostUsd ?? "—"}</div>
          </div>
        </div>
        {events.data?.items.length ? (
          <div className="border-t pt-2">
            <div className="mb-1 text-xs font-medium text-muted-foreground">
              Recent requests
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
