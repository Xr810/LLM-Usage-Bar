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

function relativeReset(reset: string): string | null {
  const timestamp = Date.parse(reset);
  if (!Number.isFinite(timestamp)) return null;
  const minutes = Math.max(0, Math.ceil((timestamp - Date.now()) / 60_000));
  if (minutes >= 24 * 60) return `${Math.ceil(minutes / (24 * 60))}d`;
  if (minutes >= 60) return `${Math.ceil(minutes / 60)}h`;
  return `${minutes}m`;
}

export function SubscriptionProviderCard({
  usage,
  onRefreshQuota,
  onSyncSessions,
  isRefreshingQuota = false,
  isSyncingSessions = false,
}: Props) {
  const { t, i18n } = useTranslation();
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
  const quotaWindow = (
    label: string,
    value: string | null | undefined,
    reset: string | null | undefined,
  ) => {
    const relative = reset ? relativeReset(reset) : null;
    const parsedReset = reset ? Date.parse(reset) : Number.NaN;
    const absoluteReset =
      reset && Number.isFinite(parsedReset)
        ? new Intl.DateTimeFormat(i18n.resolvedLanguage ?? i18n.language, {
            dateStyle: "medium",
            timeStyle: "short",
          }).format(parsedReset)
        : reset;
    return (
      <div className="rounded-lg bg-muted/40 p-3">
        <div className="text-xs text-muted-foreground">{label}</div>
        {value == null ? (
          <div className="mt-1 text-sm font-medium">
            {t("usageDashboard.quotaWindowUnavailable", {
              defaultValue:
                "This subscription does not provide this quota window",
            })}
          </div>
        ) : (
          <div className="mt-1 text-lg font-semibold">{usedLabel(value)}</div>
        )}
        {absoluteReset ? (
          <div className="text-xs text-muted-foreground">
            {t("usageDashboard.resetsAt", {
              value: absoluteReset,
              defaultValue: `Resets ${absoluteReset}`,
            })}
            {relative
              ? ` · ${t("usageDashboard.resetsIn", {
                  value: relative,
                  defaultValue: `in ${relative}`,
                })}`
              : ""}
          </div>
        ) : null}
      </div>
    );
  };

  const tokenItems = [
    [
      t("usageDashboard.inputTokens", { defaultValue: "Input" }),
      usage.inputTokens,
    ],
    [
      t("usageDashboard.outputTokens", { defaultValue: "Output" }),
      usage.outputTokens,
    ],
    [
      t("usageDashboard.cacheReadTokens", { defaultValue: "Cache read" }),
      usage.cacheReadTokens,
    ],
    [
      t("usageDashboard.cacheCreationTokens", {
        defaultValue: "Cache creation",
      }),
      usage.cacheCreationTokens,
    ],
    [t("usageDashboard.totalTokens", { defaultValue: "Total" }), totalTokens],
  ] as const;

  return (
    <Card data-testid={`subscription-provider-${usage.provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base">{usage.provider.name}</CardTitle>
          <Badge variant="secondary">
            {t("usageDashboard.subscription", { defaultValue: "Subscription" })}
          </Badge>
        </div>
        <div className="text-xs text-muted-foreground">
          {sourceText}
          {(fetchState?.lastSuccessAt ?? quota?.fetchedAt) ? (
            <>
              {" · "}
              {t("usageDashboard.lastUpdated", {
                value: new Date(
                  (fetchState?.lastSuccessAt ?? quota?.fetchedAt ?? 0) * 1000,
                ).toLocaleString(),
                defaultValue: `Last updated ${new Date(
                  (fetchState?.lastSuccessAt ?? quota?.fetchedAt ?? 0) * 1000,
                ).toLocaleString()}`,
              })}
            </>
          ) : null}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-2 sm:grid-cols-2">
          {quotaWindow(
            t("usageDashboard.fiveHour", { defaultValue: "5 hour" }),
            quota?.fiveHourUtilizationPercent,
            quota?.fiveHourResetsAt,
          )}
          {quotaWindow(
            t("usageDashboard.sevenDay", { defaultValue: "7 day" }),
            quota?.sevenDayUtilizationPercent,
            quota?.sevenDayResetsAt,
          )}
        </div>
        <div className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-5">
          {tokenItems.map(([label, value]) => (
            <div key={label}>
              <div className="text-muted-foreground">{label}</div>
              <div className="font-semibold">{value.toLocaleString()}</div>
            </div>
          ))}
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
