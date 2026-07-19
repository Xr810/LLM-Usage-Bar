import { AlertTriangle, FolderSync, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import type { ProviderUsageView } from "@/types/usageDashboard";
import { useTranslation } from "react-i18next";
import { QuotaMeter } from "./QuotaMeter";
import {
  DEFAULT_REMAINING_THRESHOLDS,
  dashboardProviderIcon,
  formatTokensCompact,
  parsePercentValue,
  type RemainingThresholds,
  toneFromRemainingPercent,
} from "./usagePresentation";

interface Props {
  usage: ProviderUsageView;
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
  remainingThresholds?: RemainingThresholds;
}

type RelativeReset = {
  count: number;
  unit: "days" | "hours" | "minutes";
};

function relativeReset(reset: string): RelativeReset | null {
  const timestamp = Date.parse(reset);
  if (!Number.isFinite(timestamp)) return null;
  const minutes = Math.max(0, Math.ceil((timestamp - Date.now()) / 60_000));
  if (minutes >= 24 * 60) {
    return { count: Math.ceil(minutes / (24 * 60)), unit: "days" };
  }
  if (minutes >= 60) {
    return { count: Math.ceil(minutes / 60), unit: "hours" };
  }
  return { count: minutes, unit: "minutes" };
}

export function SubscriptionProviderCard({
  usage,
  onRefreshQuota,
  onSyncSessions,
  isRefreshingQuota = false,
  isSyncingSessions = false,
  remainingThresholds = DEFAULT_REMAINING_THRESHOLDS,
}: Props) {
  const { t, i18n } = useTranslation();
  const canRefreshQuota = Boolean(usage.provider.quotaSource);
  const quota = canRefreshQuota ? usage.quota : null;
  const fetchState = canRefreshQuota ? usage.quotaFetchState : null;
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
  const { icon, iconColor } = dashboardProviderIcon(usage.provider);

  const quotaWindow = (
    label: string,
    value: string | null | undefined,
    reset: string | null | undefined,
  ) => {
    const relative = reset ? relativeReset(reset) : null;
    const relativeLabel = relative
      ? relative.unit === "days"
        ? t("usageDashboard.durationDays", {
            count: relative.count,
            defaultValue: `${relative.count}d`,
          })
        : relative.unit === "hours"
          ? t("usageDashboard.durationHours", {
              count: relative.count,
              defaultValue: `${relative.count}h`,
            })
          : t("usageDashboard.durationMinutes", {
              count: relative.count,
              defaultValue: `${relative.count}m`,
            })
      : null;
    const parsedReset = reset ? Date.parse(reset) : Number.NaN;
    const absoluteReset =
      reset && Number.isFinite(parsedReset)
        ? new Intl.DateTimeFormat(i18n.resolvedLanguage ?? i18n.language, {
            dateStyle: "medium",
            timeStyle: "short",
          }).format(parsedReset)
        : reset;
    const used = parsePercentValue(value);
    const remaining = used == null ? null : Math.max(0, 100 - used);
    const unavailableText = t("usageDashboard.quotaWindowUnavailable", {
      defaultValue: "This subscription does not provide this quota window",
    });
    return (
      <QuotaMeter
        key={label}
        label={label}
        meterLabel={label}
        fillPercent={remaining}
        tone={toneFromRemainingPercent(remaining, remainingThresholds)}
        valueText={
          remaining == null
            ? unavailableText
            : t("usageDashboard.remainingPercent", {
                value: `${Math.round(remaining)}`,
                defaultValue: "{{value}}% left",
              })
        }
        footer={
          absoluteReset ? (
            <span title={absoluteReset}>
              {t("usageDashboard.resetsAt", {
                value: absoluteReset,
                defaultValue: `Resets ${absoluteReset}`,
              })}
              {relativeLabel
                ? ` · ${t("usageDashboard.resetsIn", {
                    value: relativeLabel,
                    defaultValue: `in ${relativeLabel}`,
                  })}`
                : ""}
            </span>
          ) : undefined
        }
      />
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

  const lastSuccessAt = fetchState?.lastSuccessAt ?? quota?.fetchedAt;

  return (
    <Card
      data-testid={`subscription-provider-${usage.provider.id}`}
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
                {t("usageDashboard.subscription", {
                  defaultValue: "Subscription",
                })}
              </Badge>
            </div>
            <p className="mt-0.5 truncate text-xs text-muted-foreground">
              {sourceText}
              {lastSuccessAt ? (
                <>
                  {" · "}
                  {t("usageDashboard.lastUpdated", {
                    value: new Date(lastSuccessAt * 1000).toLocaleString(),
                    defaultValue: `Last updated ${new Date(
                      lastSuccessAt * 1000,
                    ).toLocaleString()}`,
                  })}
                </>
              ) : null}
            </p>
          </div>
        </div>
        {fetchState?.stale ? (
          <Badge variant="warning" role="status" className="shrink-0">
            <AlertTriangle className="h-3 w-3" aria-hidden="true" />
            {t("usageDashboard.stale", { defaultValue: "Stale" })}
          </Badge>
        ) : null}
      </div>

      <div className="grid gap-2.5 px-5 pt-4 sm:grid-cols-2">
        {quotaWindow(
          t("usageDashboard.fiveHourWindow", { defaultValue: "5-hour window" }),
          quota?.fiveHourUtilizationPercent,
          quota?.fiveHourResetsAt,
        )}
        {quotaWindow(
          t("usageDashboard.sevenDayWindow", { defaultValue: "7-day window" }),
          quota?.sevenDayUtilizationPercent,
          quota?.sevenDayResetsAt,
        )}
      </div>

      <div className="px-5 pt-4">
        <dl className="grid grid-cols-5 gap-2 rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15">
          {tokenItems.map(([label, value]) => (
            <div key={label} className="min-w-0">
              <dt className="truncate text-[11px] text-muted-foreground">
                {label}
              </dt>
              <dd
                className="mt-0.5 truncate text-sm font-semibold metric"
                title={value.toLocaleString()}
              >
                {formatTokensCompact(value)}
              </dd>
            </div>
          ))}
        </dl>
      </div>

      {fetchState?.stale && fetchState.lastError ? (
        <p className="px-5 pt-2 text-xs text-warning">
          {t("usageDashboard.stale", { defaultValue: "Stale" })}:{" "}
          {fetchState.lastError}
        </p>
      ) : null}
      {quota?.manualResetsRemaining != null ? (
        <p className="px-5 pt-2 text-xs text-muted-foreground">
          {t("usageDashboard.manualResets", {
            defaultValue: "Manual resets",
          })}
          : {quota.manualResetsRemaining}
        </p>
      ) : null}

      <div className="mt-4 flex items-center gap-2 border-t border-border/60 px-5 py-3">
        {canRefreshQuota ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-8 px-2.5 text-xs"
            disabled={isRefreshingQuota}
            onClick={() => void onRefreshQuota(usage.provider.id)}
          >
            <RefreshCw
              className={
                isRefreshingQuota ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"
              }
              aria-hidden="true"
            />
            {t("usageDashboard.refreshQuota", {
              defaultValue: "Refresh quota",
            })}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="h-8 px-2.5 text-xs"
          disabled={isSyncingSessions}
          onClick={() => void onSyncSessions(usage.provider.id)}
        >
          <FolderSync className="h-3.5 w-3.5" aria-hidden="true" />
          {t("usageDashboard.syncSessions", {
            defaultValue: "Sync sessions",
          })}
        </Button>
      </div>
    </Card>
  );
}
