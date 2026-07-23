import { useTranslation } from "react-i18next";
import { PlugZap } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import type { ProviderDashboardProjection } from "./usageDashboardProjection";
import { MeteredProviderCard } from "./MeteredProviderCard";
import { ProviderActivityHeatmap } from "./ProviderActivityHeatmap";
import { SubscriptionProviderCard } from "./SubscriptionProviderCard";
import { ProviderUsageTrendChart } from "./ProviderUsageTrendChart";
import { formatTokensCompact } from "./usagePresentation";
import type { RemainingThresholds } from "./usagePresentation";

interface ProviderUsagePageProps {
  projection: ProviderDashboardProjection;
  startAt: number;
  endAt: number;
  rangeLabel: string;
  activityBuckets: UsageTrendBucketView[];
  activityStartAt: number;
  activityEndAt: number;
  isActivityLoading?: boolean;
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  onOpenSettings?: () => void;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
  remainingThresholds?: RemainingThresholds;
}

export function ProviderUsagePage({
  projection,
  startAt,
  endAt,
  rangeLabel,
  activityBuckets,
  activityStartAt,
  activityEndAt,
  isActivityLoading = false,
  onRefreshQuota,
  onSyncSessions,
  onOpenSettings,
  isRefreshingQuota = false,
  isSyncingSessions = false,
  remainingThresholds,
}: ProviderUsagePageProps) {
  const { t } = useTranslation();
  const costText =
    projection.meteredTotalCostUsd == null
      ? t("usageDashboard.costUnavailableSummary", {
          defaultValue: "Cost unavailable",
        })
      : `$${projection.meteredTotalCostUsd}`;
  const costStatusText =
    projection.meteredCostStatus === "partial"
      ? t("usageDashboard.costPartial", { defaultValue: "Partial cost data" })
      : projection.meteredCostStatus === "estimated"
        ? t("usageDashboard.costEstimatedSummary", {
            defaultValue: "Includes estimated cost",
          })
        : projection.meteredCostStatus === "unavailable"
          ? t("usageDashboard.costUnavailableSummary", {
              defaultValue: "Cost unavailable",
            })
          : t("usageDashboard.costComplete", {
              defaultValue: "Trusted cost data",
            });

  if (
    projection.subscriptionProviders.length === 0 &&
    projection.meteredProviders.length === 0
  ) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-muted/20 px-8 py-14 text-center dark:bg-muted/10">
        <div className="mx-auto mb-3 grid h-10 w-10 place-items-center rounded-full bg-muted text-muted-foreground">
          <PlugZap className="h-5 w-5" aria-hidden="true" />
        </div>
        <p className="text-sm font-medium">
          {t("usageDashboard.providerEmpty", {
            defaultValue: "No Provider accounts are configured.",
          })}
        </p>
        {onOpenSettings ? (
          <Button className="mt-4" size="sm" onClick={onOpenSettings}>
            {t("usageDashboard.configureProviders", {
              defaultValue: "Configure Providers in Settings",
            })}
          </Button>
        ) : null}
      </div>
    );
  }

  const sectionHeading = (id: string, text: string, count: number) => (
    <div className="flex items-baseline gap-2">
      <h2
        id={id}
        className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
      >
        {text}
      </h2>
      <span className="text-xs metric text-muted-foreground/70">{count}</span>
    </div>
  );

  return (
    <div className="space-y-7">
      <div className="grid items-start gap-4 min-[900px]:grid-cols-[300px_minmax(0,1fr)] min-[1180px]:grid-cols-[330px_minmax(0,1fr)]">
        <section
          className="overflow-hidden rounded-xl border bg-card px-4 shadow-card"
          aria-labelledby="subscription-heading"
        >
          <div className="flex items-start justify-between gap-3 py-4">
            <div>
              <h2 id="subscription-heading" className="text-sm font-semibold">
                {t("usageDashboard.remainingQuota", {
                  defaultValue: "Remaining quota",
                })}
              </h2>
              <p className="mt-1 text-[11px] text-muted-foreground">
                {t("usageDashboard.subscriptionAccounts", {
                  defaultValue: "Subscription accounts",
                })}
              </p>
            </div>
            <span className="shrink-0 text-xs text-muted-foreground metric">
              {t("usageDashboard.accountCount", {
                count: projection.subscriptionProviders.length,
                defaultValue: "{{count}} accounts",
              })}
            </span>
          </div>

          {projection.subscriptionProviders.length ? (
            <div>
              {projection.subscriptionProviders.map((usage) => (
                <SubscriptionProviderCard
                  key={usage.provider.id}
                  usage={usage}
                  layout="sidebar"
                  onRefreshQuota={onRefreshQuota}
                  onSyncSessions={onSyncSessions}
                  isRefreshingQuota={isRefreshingQuota}
                  isSyncingSessions={isSyncingSessions}
                  remainingThresholds={remainingThresholds}
                />
              ))}
            </div>
          ) : (
            <div className="border-t border-dashed border-border py-8 text-center text-sm text-muted-foreground">
              {t("usageDashboard.noSubscriptionProviders", {
                defaultValue: "No subscription Provider accounts.",
              })}
            </div>
          )}

          <dl className="grid grid-cols-2 gap-2 border-t border-border/60 py-4">
            <div className="rounded-lg bg-muted/20 px-3 py-2.5">
              <dt className="text-[10px] text-muted-foreground">
                {t("usageDashboard.selectedRangeTokens", {
                  defaultValue: "Selected range",
                })}
              </dt>
              <dd
                className="mt-1 text-base font-semibold metric"
                title={projection.overallTotalTokens.toLocaleString()}
              >
                {formatTokensCompact(projection.overallTotalTokens)}
              </dd>
            </div>
            <div className="rounded-lg bg-muted/20 px-3 py-2.5">
              <dt className="text-[10px] text-muted-foreground">
                {t("usageDashboard.records", { defaultValue: "Records" })}
              </dt>
              <dd className="mt-1 text-base font-semibold metric">
                {projection.overallRequestCount.toLocaleString()}
              </dd>
            </div>
          </dl>
        </section>

        <div className="min-w-0 space-y-4">
          <ProviderActivityHeatmap
            buckets={activityBuckets}
            startAt={activityStartAt}
            endAt={activityEndAt}
            isLoading={isActivityLoading}
          />
          <ProviderUsageTrendChart
            granularity={projection.trendGranularity}
            buckets={projection.trendBuckets}
            totalTokens={projection.overallTotalTokens}
            recordCount={projection.overallRequestCount}
            rangeLabel={rangeLabel}
          />
        </div>
      </div>

      <section className="space-y-3" aria-labelledby="metered-heading">
        {sectionHeading(
          "metered-heading",
          t("usageDashboard.meteredProviders", {
            defaultValue: "Metered accounts",
          }),
          projection.meteredProviders.length,
        )}
        <Card className="overflow-hidden">
          <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 px-5 py-4">
            <div className="flex items-center gap-2.5">
              <h3 className="text-sm font-semibold tracking-tight">
                {t("usageDashboard.meteredOverview", {
                  defaultValue: "Metered overview",
                })}
              </h3>
              <Badge variant="outline">{costStatusText}</Badge>
            </div>
            <dl className="flex items-center gap-6">
              <div className="min-w-0">
                <dt className="text-[11px] text-muted-foreground">
                  {t("usageDashboard.tokens", { defaultValue: "Tokens" })}
                </dt>
                <dd
                  className="mt-0.5 text-base font-semibold metric"
                  data-testid="metered-total-tokens"
                  title={projection.meteredTotalTokens.toLocaleString()}
                >
                  {formatTokensCompact(projection.meteredTotalTokens)}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[11px] text-muted-foreground">
                  {t("usageDashboard.requests", { defaultValue: "Requests" })}
                </dt>
                <dd
                  className="mt-0.5 text-base font-semibold metric"
                  data-testid="metered-request-count"
                >
                  {projection.meteredRequestCount.toLocaleString()}
                </dd>
              </div>
              <div className="min-w-0">
                <dt className="text-[11px] text-muted-foreground">USD</dt>
                <dd
                  className="mt-0.5 text-base font-semibold metric"
                  data-testid="metered-total-cost"
                >
                  {costText}
                </dd>
              </div>
            </dl>
          </div>
        </Card>
        {projection.meteredProviders.length ? (
          <div className="grid gap-4 lg:grid-cols-2">
            {projection.meteredProviders.map((usage) => (
              <MeteredProviderCard
                key={usage.provider.id}
                usage={usage}
                startAt={startAt}
                endAt={endAt}
              />
            ))}
          </div>
        ) : (
          <div className="rounded-xl border border-dashed border-border bg-muted/20 px-4 py-6 text-center text-sm text-muted-foreground dark:bg-muted/10">
            {t("usageDashboard.noMeteredProviders", {
              defaultValue: "No metered Provider accounts.",
            })}
          </div>
        )}
      </section>
    </div>
  );
}
