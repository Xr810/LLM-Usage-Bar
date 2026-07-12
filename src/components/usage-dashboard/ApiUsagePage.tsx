import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ApiModuleProjection } from "./usageDashboardProjection";
import { MeteredProviderCard } from "./MeteredProviderCard";

interface ApiUsagePageProps {
  projection: ApiModuleProjection;
  startAt: number;
  endAt: number;
  onOpenSettings?: () => void;
}

export function ApiUsagePage({
  projection,
  startAt,
  endAt,
  onOpenSettings,
}: ApiUsagePageProps) {
  const { t } = useTranslation();
  const costText =
    projection.totalCostUsd == null
      ? t("usageDashboard.costUnavailable", {
          defaultValue: "Cost unavailable",
        })
      : `$${projection.totalCostUsd}`;
  const costStatusText =
    projection.costStatus === "partial"
      ? t("usageDashboard.costPartial", {
          defaultValue: "Partial cost data",
        })
      : projection.costStatus === "estimated"
        ? t("usageDashboard.costEstimatedSummary", {
            defaultValue: "Includes estimated cost",
          })
        : projection.costStatus === "unavailable"
          ? t("usageDashboard.costUnavailable", {
              defaultValue: "Cost unavailable",
            })
          : t("usageDashboard.costComplete", {
              defaultValue: "Trusted cost data",
            });

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between gap-3">
            <CardTitle className="text-base">
              {t("usageDashboard.apiOverview", {
                defaultValue: "API overview",
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
            <div
              className="text-xl font-semibold"
              data-testid="api-total-tokens"
            >
              {projection.totalTokens.toLocaleString()}
            </div>
          </div>
          <div>
            <div className="text-xs text-muted-foreground">
              {t("usageDashboard.requests", { defaultValue: "Requests" })}
            </div>
            <div
              className="text-xl font-semibold"
              data-testid="api-request-count"
            >
              {projection.requestCount.toLocaleString()}
            </div>
          </div>
          <div>
            <div className="text-xs text-muted-foreground">USD</div>
            <div className="text-xl font-semibold" data-testid="api-total-cost">
              {costText}
            </div>
          </div>
        </CardContent>
      </Card>

      <div className="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
        {t("usageDashboard.aggregateUnavailable", {
          defaultValue:
            "Cross-provider model distribution is unavailable until the desktop data service provides it.",
        })}
      </div>

      {projection.providers.length ? (
        <div className="grid gap-4 lg:grid-cols-2">
          {projection.providers.map((usage) => (
            <MeteredProviderCard
              key={usage.provider.id}
              usage={usage}
              startAt={startAt}
              endAt={endAt}
            />
          ))}
        </div>
      ) : (
        <div className="rounded-lg border border-dashed p-8 text-center">
          <p className="text-sm text-muted-foreground">
            {t("usageDashboard.apiModuleEmpty", {
              defaultValue: "No API Provider is configured.",
            })}
          </p>
          {onOpenSettings ? (
            <Button className="mt-3" size="sm" onClick={onOpenSettings}>
              {t("usageDashboard.addProviderInSettings", {
                defaultValue: "Add Provider in Settings",
              })}
            </Button>
          ) : null}
        </div>
      )}
    </div>
  );
}
