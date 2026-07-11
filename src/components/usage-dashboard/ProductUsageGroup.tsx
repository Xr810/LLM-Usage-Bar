import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ProductUsageView } from "@/types/usageDashboard";
import { MeteredProviderCard } from "./MeteredProviderCard";
import { SubscriptionProviderCard } from "./SubscriptionProviderCard";
import { useTranslation } from "react-i18next";

interface Props {
  product: ProductUsageView;
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  startAt: number;
  endAt: number;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
}

export function ProductUsageGroup({
  product,
  onRefreshQuota,
  onSyncSessions,
  startAt,
  endAt,
  isRefreshingQuota = false,
  isSyncingSessions = false,
}: Props) {
  const { t } = useTranslation();
  const totalTokens =
    product.inputTokens +
    product.outputTokens +
    product.cacheReadTokens +
    product.cacheCreationTokens;
  const sourceText = product.tokenSources
    .map((source) =>
      source === "proxy"
        ? t("usageDashboard.sourceProxy", { defaultValue: "Proxy" })
        : t("usageDashboard.sourceSession", { defaultValue: "Session log" }),
    )
    .join(" + ");
  return (
    <Card className="border-border/60">
      <CardHeader>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle>{product.productGroupId}</CardTitle>
            <div className="mt-1 text-sm text-muted-foreground">
              {t("usageDashboard.sources", { defaultValue: "Sources" })}:{" "}
              {sourceText}
            </div>
          </div>
          <div className="text-right">
            <div className="text-2xl font-semibold">
              {totalTokens.toLocaleString()}
            </div>
            <div className="text-xs text-muted-foreground">
              {t("usageDashboard.tokens", { defaultValue: "Tokens" })} · USD{" "}
              {product.totalCostUsd ?? "—"}
            </div>
          </div>
        </div>
        <div className="flex flex-wrap gap-2 pt-2">
          <Badge variant="outline">
            <span>
              {t("usageDashboard.costUpstream", { defaultValue: "Upstream" })}
            </span>
            &nbsp;{product.costSourceCounts.upstream}
          </Badge>
          <Badge variant="outline">
            <span>
              {t("usageDashboard.costEstimated", { defaultValue: "Estimated" })}
            </span>
            &nbsp;{product.costSourceCounts.estimated}
          </Badge>
          <Badge variant="outline">
            <span>
              {t("usageDashboard.costUnavailable", {
                defaultValue: "Unavailable",
              })}
            </span>
            &nbsp;{product.costSourceCounts.unavailable}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 lg:grid-cols-2">
        {product.subscriptionProviders.map((usage) => (
          <SubscriptionProviderCard
            key={usage.provider.id}
            usage={usage}
            onRefreshQuota={onRefreshQuota}
            onSyncSessions={onSyncSessions}
            isRefreshingQuota={isRefreshingQuota}
            isSyncingSessions={isSyncingSessions}
          />
        ))}
        {product.meteredProviders.map((usage) => (
          <MeteredProviderCard
            key={usage.provider.id}
            usage={usage}
            startAt={startAt}
            endAt={endAt}
          />
        ))}
      </CardContent>
    </Card>
  );
}
