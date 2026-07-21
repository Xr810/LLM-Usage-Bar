import { AlertTriangle, FolderSync, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ManualResetCredits } from "@/components/ManualResetCredits";
import { cn } from "@/lib/utils";
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
  layout?: "default" | "sidebar";
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
  layout = "default",
}: Props) {
  const { t, i18n } = useTranslation();
  const sidebar = layout === "sidebar";
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
        flat={sidebar}
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

  const tokenItems = sidebar
    ? [
        {
          label: t("usageDashboard.totalTokens", { defaultValue: "Total" }),
          value: totalTokens,
          text: formatTokensCompact(totalTokens),
        },
        {
          label: t("usageDashboard.records", { defaultValue: "Records" }),
          value: usage.eventCount,
          text: usage.eventCount.toLocaleString(
            i18n.resolvedLanguage ?? i18n.language,
          ),
        },
      ]
    : [
        {
          label: t("usageDashboard.inputTokens", { defaultValue: "Input" }),
          value: usage.inputTokens,
          text: formatTokensCompact(usage.inputTokens),
        },
        {
          label: t("usageDashboard.outputTokens", { defaultValue: "Output" }),
          value: usage.outputTokens,
          text: formatTokensCompact(usage.outputTokens),
        },
        {
          label: t("usageDashboard.cacheReadTokens", {
            defaultValue: "Cache read",
          }),
          value: usage.cacheReadTokens,
          text: formatTokensCompact(usage.cacheReadTokens),
        },
        {
          label: t("usageDashboard.cacheCreationTokens", {
            defaultValue: "Cache creation",
          }),
          value: usage.cacheCreationTokens,
          text: formatTokensCompact(usage.cacheCreationTokens),
        },
        {
          label: t("usageDashboard.totalTokens", { defaultValue: "Total" }),
          value: totalTokens,
          text: formatTokensCompact(totalTokens),
        },
      ];

  const lastSuccessAt = fetchState?.lastSuccessAt ?? quota?.fetchedAt;

  return (
    <Card
      data-testid={`subscription-provider-${usage.provider.id}`}
      data-layout={layout}
      className={cn(
        "overflow-hidden",
        sidebar &&
          "rounded-none border-x-0 border-b-0 bg-transparent shadow-none",
      )}
    >
      <div
        className={cn(
          "flex items-center justify-between gap-3 pt-4",
          sidebar ? "px-0" : "px-5",
        )}
      >
        <div className="flex min-w-0 items-center gap-3">
          <ProviderIcon
            icon={icon}
            color={iconColor}
            name={usage.provider.name}
            size={sidebar ? 30 : 34}
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

      <div
        className={cn(
          "grid gap-2.5 pt-4",
          sidebar ? "grid-cols-1 px-0" : "px-5 sm:grid-cols-2",
        )}
      >
        {quotaWindow(
          t("usageDashboard.fiveHourWindow", { defaultValue: "5-hour window" }),
          quota?.fiveHourUtilizationPercent,
          quota?.fiveHourResetsAt,
        )}
        {quotaWindow(
          t("usageDashboard.sevenDayWindow", {
            defaultValue: "Weekly allowance",
          }),
          quota?.sevenDayUtilizationPercent,
          quota?.sevenDayResetsAt,
        )}
      </div>

      <div className={cn("pt-2", sidebar ? "px-0" : "px-5")}>
        <ManualResetCredits
          availableCount={quota?.manualResetsRemaining ?? null}
          credits={quota?.manualResetCredits ?? []}
          compact={sidebar}
        />
      </div>

      <div className={cn("pt-4", sidebar ? "px-0" : "px-5")}>
        <dl
          className={cn(
            "grid gap-2 rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15",
            sidebar ? "grid-cols-2" : "grid-cols-5",
          )}
        >
          {tokenItems.map(({ label, value, text }) => (
            <div key={label} className="min-w-0">
              <dt className="truncate text-[11px] text-muted-foreground">
                {label}
              </dt>
              <dd
                className="mt-0.5 truncate text-sm font-semibold metric"
                title={value.toLocaleString()}
              >
                {text}
              </dd>
            </div>
          ))}
        </dl>
      </div>

      {fetchState?.stale && fetchState.lastError ? (
        <p
          className={cn("pt-2 text-xs text-warning", sidebar ? "px-0" : "px-5")}
        >
          {t("usageDashboard.stale", { defaultValue: "Stale" })}:{" "}
          {fetchState.lastError}
        </p>
      ) : null}
      <div
        className={cn(
          "mt-4 flex flex-wrap items-center gap-2 border-t border-border/60 py-3",
          sidebar ? "px-0" : "px-5",
        )}
      >
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
