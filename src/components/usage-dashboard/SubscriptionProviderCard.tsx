import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { useTranslation } from "react-i18next";

interface Props {
  usage: ProviderUsageView;
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
}

function quotaLine(
  label: string,
  value: string | null,
  reset: string | null,
  usedLabel: (value: string) => string,
) {
  return (
    <div className="rounded-lg bg-muted/40 p-3">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 text-lg font-semibold">
        {value == null ? "—" : usedLabel(value)}
      </div>
      {reset ? (
        <div className="text-xs text-muted-foreground">{reset}</div>
      ) : null}
    </div>
  );
}

export function SubscriptionProviderCard({
  usage,
  onRefreshQuota,
  onSyncSessions,
  isRefreshingQuota = false,
  isSyncingSessions = false,
}: Props) {
  const { t } = useTranslation();
  const quota = usage.quota;
  const fetchState = usage.quotaFetchState;
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
  const usedLabel = (value: string) =>
    t("usageDashboard.usedPercent", {
      value,
      defaultValue: `${value}% used`,
    });
  return (
    <Card data-testid={`subscription-provider-${usage.provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base">{usage.provider.name}</CardTitle>
          <Badge variant="secondary">
            {t("usageDashboard.subscription", { defaultValue: "Subscription" })}
          </Badge>
        </div>
        <div className="text-xs text-muted-foreground">{sourceText}</div>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-2 gap-2">
          {quotaLine(
            t("usageDashboard.fiveHour", { defaultValue: "5 hour" }),
            quota?.fiveHourUtilizationPercent ?? null,
            quota?.fiveHourResetsAt ?? null,
            usedLabel,
          )}
          {quotaLine(
            t("usageDashboard.sevenDay", { defaultValue: "7 day" }),
            quota?.sevenDayUtilizationPercent ?? null,
            quota?.sevenDayResetsAt ?? null,
            usedLabel,
          )}
        </div>
        <div className="grid grid-cols-3 gap-3 text-sm">
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
        {quota?.manualResetsRemaining != null ? (
          <div className="text-sm text-muted-foreground">
            {t("usageDashboard.manualResets", {
              defaultValue: "Manual resets",
            })}
            : {quota.manualResetsRemaining}
          </div>
        ) : null}
        {fetchState?.stale ? (
          <div role="status" className="text-sm text-amber-600">
            {t("usageDashboard.stale", { defaultValue: "Stale" })}:{" "}
            {fetchState.lastError ??
              t("common.unknown", { defaultValue: "Unknown" })}
          </div>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={isRefreshingQuota}
            onClick={() => void onRefreshQuota(usage.provider.id)}
          >
            {t("usageDashboard.refreshQuota", {
              defaultValue: "Refresh quota",
            })}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={isSyncingSessions}
            onClick={() => void onSyncSessions(usage.provider.id)}
          >
            {t("usageDashboard.syncSessions", {
              defaultValue: "Sync sessions",
            })}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
