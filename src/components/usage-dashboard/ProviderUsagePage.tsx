import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ProviderDashboardProjection } from "./usageDashboardProjection";
import { MeteredProviderCard } from "./MeteredProviderCard";
import { SubscriptionProviderCard } from "./SubscriptionProviderCard";

interface ProviderUsagePageProps {
  projection: ProviderDashboardProjection;
  startAt: number;
  endAt: number;
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  onOpenSettings?: () => void;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
}

export function ProviderUsagePage({
  projection,
  startAt,
  endAt,
  onRefreshQuota,
  onSyncSessions,
  onOpenSettings,
  isRefreshingQuota = false,
  isSyncingSessions = false,
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
      <div className="rounded-lg border border-dashed p-8 text-center">
        <p className="text-sm text-muted-foreground">
          {t("usageDashboard.providerEmpty", {
            defaultValue: "No Provider accounts are configured.",
          })}
        </p>
        {onOpenSettings ? (
          <Button className="mt-3" size="sm" onClick={onOpenSettings}>
            {t("usageDashboard.configureProviders", {
              defaultValue: "Configure Providers in Settings",
            })}
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <section className="space-y-3" aria-labelledby="subscription-heading">
        <h2 id="subscription-heading" className="text-base font-semibold">
          {t("usageDashboard.subscriptionAccounts", {
            defaultValue: "Subscription accounts",
          })}
        </h2>
        {projection.subscriptionProviders.length ? (
          <div className="grid gap-4 lg:grid-cols-2">
            {projection.subscriptionProviders.map((usage) => (
              <SubscriptionProviderCard
                key={usage.provider.id}
                usage={usage}
                onRefreshQuota={onRefreshQuota}
                onSyncSessions={onSyncSessions}
                isRefreshingQuota={isRefreshingQuota}
                isSyncingSessions={isSyncingSessions}
              />
            ))}
          </div>
        ) : (
          <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            {t("usageDashboard.noSubscriptionProviders", {
              defaultValue: "No subscription Provider accounts.",
            })}
          </div>
        )}
      </section>

      <section className="space-y-3" aria-labelledby="metered-heading">
        <h2 id="metered-heading" className="text-base font-semibold">
          {t("usageDashboard.meteredProviders", {
            defaultValue: "Metered Provider accounts",
          })}
        </h2>
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between gap-3">
              <CardTitle className="text-base">
                {t("usageDashboard.meteredOverview", {
                  defaultValue: "Metered overview",
                })}
              </CardTitle>
              <Badge variant="outline">{costStatusText}</Badge>
            </div>
          </CardHeader>
          <CardContent className="grid gap-4 sm:grid-cols-3">
            <div>
              <div className="text-xs text-muted-foreground">
                {t("usageDashboard.tokens", { defaultValue: "Tokens" })}
              </div>
              <div className="text-xl font-semibold" data-testid="metered-total-tokens">
                {projection.meteredTotalTokens.toLocaleString()}
              </div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground">
                {t("usageDashboard.requests", { defaultValue: "Requests" })}
              </div>
              <div className="text-xl font-semibold" data-testid="metered-request-count">
                {projection.meteredRequestCount.toLocaleString()}
              </div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground">USD</div>
              <div className="text-xl font-semibold" data-testid="metered-total-cost">
                {costText}
              </div>
            </div>
          </CardContent>
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
          <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            {t("usageDashboard.noMeteredProviders", {
              defaultValue: "No metered Provider accounts.",
            })}
          </div>
        )}
      </section>
    </div>
  );
}
