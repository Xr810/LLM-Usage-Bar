import { AlertTriangle, FolderSync, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ManualResetCredits } from "@/components/ManualResetCredits";
import { cn } from "@/lib/utils";
import { relativeTimeAgo } from "@/lib/relativeTime";
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
  /**
   * `compact` is the monitoring dashboard's grid tile: same quota detail, no
   * manual actions, and a tighter footprint so several accounts fit side by side.
   */
  layout?: "default" | "compact";
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
  const compact = layout === "compact";
  const canRefreshQuota = Boolean(usage.provider.quotaSource);
  const quota = canRefreshQuota ? usage.quota : null;
  const fetchState = canRefreshQuota ? usage.quotaFetchState : null;
  const totalTokens =
    usage.inputTokens +
    usage.outputTokens +
    usage.cacheReadTokens +
    usage.cacheCreationTokens;
  const { icon, iconColor } = dashboardProviderIcon(usage.provider);

  // The plan comes from the credential, so it is only known for Providers whose
  // token carries one. "pro" is displayed as "Pro"; anything longer is left as
  // the upstream wrote it beyond the first letter, since "Team" and
  // "Enterprise" are the upstream's own casing and not ours to restyle.
  const planLabel = quota?.planType
    ? quota.planType.charAt(0).toUpperCase() + quota.planType.slice(1)
    : null;
  const planRenewsLabel =
    planLabel && quota?.planRenewsAt
      ? t("usageDashboard.planRenewsAt", {
          value: new Date(quota.planRenewsAt * 1000).toLocaleDateString(
            i18n.resolvedLanguage ?? i18n.language,
            { year: "numeric", month: "long", day: "numeric" },
          ),
          defaultValue: `Renews ${new Date(
            quota.planRenewsAt * 1000,
          ).toLocaleDateString()}`,
        })
      : null;

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
        flat={compact}
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
            // The exact timestamp is long and rarely what you want at a glance;
            // lead with "in 2h" and keep the absolute time on hover.
            <span title={absoluteReset}>
              {relativeLabel
                ? t("usageDashboard.resetsIn", {
                    value: relativeLabel,
                    defaultValue: `in ${relativeLabel}`,
                  })
                : t("usageDashboard.resetsAt", {
                    value: absoluteReset,
                    defaultValue: `Resets ${absoluteReset}`,
                  })}
            </span>
          ) : undefined
        }
      />
    );
  };

  const tokenItems = compact
    ? [
        {
          // Named rather than "Total": on a compact card it stands beside the
          // call count, where a bare "Total" reads as totalling those.
          label: t("usageDashboard.totalTokensNamed", {
            defaultValue: "Tokens",
          }),
          value: totalTokens,
          text: formatTokensCompact(totalTokens),
        },
        {
          label: t("usageDashboard.records", { defaultValue: "Calls" }),
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

  const lastSuccessAt =
    usage.provider.quotaSource === "claude_local"
      ? (quota?.sourceObservedAt ?? quota?.fetchedAt)
      : (fetchState?.lastSuccessAt ?? quota?.fetchedAt);
  // "3 minutes ago" answers "is this current?" at a glance; the exact
  // timestamp stays available as the line's tooltip.
  const freshnessLabel = lastSuccessAt
    ? relativeTimeAgo(
        lastSuccessAt * 1000,
        i18n.resolvedLanguage ?? i18n.language,
      )
    : "";
  const hasManualResets =
    quota?.manualResetsRemaining != null ||
    (quota?.manualResetCredits?.length ?? 0) > 0;

  return (
    <Card
      data-testid={`subscription-provider-${usage.provider.id}`}
      data-layout={layout}
      className={cn("flex flex-col overflow-hidden", compact && "h-full")}
    >
      <div className="flex items-center justify-between gap-3 px-5 pt-4">
        <div className="flex min-w-0 items-center gap-3">
          <ProviderIcon
            icon={icon}
            color={iconColor}
            name={usage.provider.name}
            size={compact ? 30 : 34}
            className="shrink-0 rounded-[10px] border border-border/50"
          />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="truncate text-[15px] font-semibold tracking-tight">
                {usage.provider.name}
              </h3>
              {/* The plan replaces the generic "Subscription" rather than
                  joining it: this Provider is named "ChatGPT Plus/Pro"
                  precisely because the app could not tell which, so once it
                  can, that is the badge worth the space. */}
              <Badge
                variant="secondary"
                className="shrink-0"
                title={planRenewsLabel ?? undefined}
              >
                {planLabel ??
                  t("usageDashboard.subscription", {
                    defaultValue: "Subscription",
                  })}
              </Badge>
            </div>
            {/* Where the numbers came from used to lead this line. It named
                every source and caveat on every card, on every render, to
                answer a question asked once — the freshness is what actually
                changes and is what the line is for now. */}
            {lastSuccessAt ? (
              <p
                data-testid="provider-provenance"
                className="mt-0.5 truncate text-xs text-muted-foreground"
                title={new Date(lastSuccessAt * 1000).toLocaleString()}
              >
                {t("usageDashboard.lastUpdated", {
                  value: freshnessLabel,
                  defaultValue: `Last updated ${freshnessLabel}`,
                })}
              </p>
            ) : null}
          </div>
        </div>
        {fetchState?.stale ? (
          <Badge variant="warning" role="status" className="shrink-0">
            <AlertTriangle className="h-3 w-3" aria-hidden="true" />
            {t("usageDashboard.stale", { defaultValue: "Stale" })}
          </Badge>
        ) : null}
      </div>

      {/* Stacked, not side by side: two meters sharing a card's width left
          each bar barely wider than its own label, and the reset caption
          under one column read as belonging to both. */}
      <div className="grid gap-2.5 px-5 pt-4">
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

      {/* ManualResetCredits renders nothing for plans without manual resets —
          skip the wrapper too, or its padding leaves a gap in the card. */}
      {hasManualResets ? (
        <div className="px-5 pt-2">
          <ManualResetCredits
            availableCount={quota?.manualResetsRemaining ?? null}
            credits={quota?.manualResetCredits ?? []}
            compact={compact}
          />
        </div>
      ) : null}

      {/* pb-5 matches the card's horizontal padding. A compact card ends here,
          so without it the block sits on the bottom border. */}
      <div className="mt-auto px-5 pb-5 pt-4">
        <dl
          className={cn(
            "grid gap-2 rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15",
            compact ? "grid-cols-2" : "grid-cols-5",
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
        <p className="px-5 pt-2 text-xs text-warning">
          {t("usageDashboard.stale", { defaultValue: "Stale" })}:{" "}
          {fetchState.lastError}
        </p>
      ) : null}
      {!compact ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-border/60 px-5 py-3">
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
      ) : null}
    </Card>
  );
}
