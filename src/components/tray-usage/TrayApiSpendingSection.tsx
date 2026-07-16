import { ExternalLink, PlusCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ProviderIcon } from "@/components/ProviderIcon";
import { Button } from "@/components/ui/button";
import {
  clampPercentForProgress,
  costQualityLabel,
  formatTokenCount,
  formatUsd,
  hasUsableBudget,
  hasUsablePercent,
  hasUsableUsd,
  providerIconName,
  type TrayProviderRow,
} from "./trayUsagePresentation";
import { TrayUsageProgress, TrayUsageStatusBadge } from "./TrayUsageProgress";

export function TrayApiSpendingSection({
  rows,
  locale,
  showAgentName,
  onOpenDetails,
  onOpenSettings,
}: {
  rows: TrayProviderRow[];
  locale: string;
  showAgentName: boolean;
  onOpenDetails: (agentModuleId: string | null) => void;
  onOpenSettings: (providerId: string | null) => void;
}) {
  const { t } = useTranslation();
  if (rows.length === 0) return null;

  return (
    <section aria-labelledby="tray-api-spending-heading">
      <h2
        id="tray-api-spending-heading"
        className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
      >
        {t("trayUsage.apiSpending", { defaultValue: "API spending" })}
      </h2>
      <div className="divide-y divide-border rounded-lg border border-border">
        {rows.map(({ agentModuleId, agentName, provider }) => {
          if (!provider.metered) return null;
          const usage = provider.metered;
          const todayCost =
            usage.costQuality === "unavailable" ? null : usage.todayCostUsd;
          const rollingCost =
            usage.costQuality === "unavailable"
              ? null
              : usage.rolling30DayCostUsd;
          const hasBudget = hasUsableBudget(usage.dailyBudgetUsd);
          const hasUsableBudgetProgress =
            hasBudget &&
            usage.costQuality !== "unavailable" &&
            hasUsableUsd(todayCost) &&
            hasUsablePercent(usage.budgetConsumedPercent);
          const quality = costQualityLabel(usage.costQuality, t);

          return (
            <article
              key={`${agentModuleId}:${provider.providerId}`}
              className="tray-usage-provider-row space-y-3 px-3 py-3"
            >
              <div className="flex min-w-0 items-start justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  <ProviderIcon
                    icon={providerIconName(provider.systemPresetKey)}
                    name={provider.providerName}
                    size={26}
                    className="rounded-md"
                  />
                  <div className="min-w-0">
                    <h3
                      className="truncate text-sm font-semibold"
                      title={provider.providerName}
                    >
                      {provider.providerName}
                    </h3>
                    {showAgentName ? (
                      <p className="truncate text-[11px] text-muted-foreground">
                        {agentName}
                      </p>
                    ) : null}
                  </div>
                </div>
                <TrayUsageStatusBadge status={provider.status} t={t} />
              </div>

              <dl className="grid grid-cols-3 gap-x-3">
                <div className="min-w-0">
                  <dt className="truncate text-[11px] text-muted-foreground">
                    {t("trayUsage.today", { defaultValue: "Today" })}
                  </dt>
                  <dd className="mt-0.5 truncate text-sm font-semibold tabular-nums">
                    {formatUsd(todayCost, locale)}
                  </dd>
                </div>
                <div className="min-w-0">
                  <dt className="truncate text-[11px] text-muted-foreground">
                    {t("trayUsage.rolling30Days", {
                      defaultValue: "Last 30 days",
                    })}
                  </dt>
                  <dd className="mt-0.5 truncate text-sm font-semibold tabular-nums">
                    {formatUsd(rollingCost, locale)}
                  </dd>
                </div>
                <div className="min-w-0">
                  <dt className="truncate text-[11px] text-muted-foreground">
                    {t("trayUsage.tokens", { defaultValue: "Tokens" })}
                  </dt>
                  <dd className="mt-0.5 truncate text-sm font-semibold tabular-nums">
                    {formatTokenCount(usage.totalTokens, locale)}
                  </dd>
                </div>
              </dl>

              <div className="space-y-1.5">
                <div className="flex items-center justify-between gap-3 text-xs">
                  <span className="font-medium">
                    {t("trayUsage.dailyBudget", {
                      defaultValue: "Daily budget",
                    })}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    {quality}
                  </span>
                </div>

                {hasBudget ? (
                  <>
                    <div className="text-sm font-semibold tabular-nums">
                      {t("trayUsage.ofBudget", {
                        cost: formatUsd(todayCost, locale),
                        budget: formatUsd(usage.dailyBudgetUsd, locale),
                        defaultValue: "{{cost}} of {{budget}}",
                      })}
                    </div>
                    {hasUsableBudgetProgress ? (
                      <TrayUsageProgress
                        label={`${t("trayUsage.dailyBudget", {
                          defaultValue: "Daily budget",
                        })} ${provider.providerName}`}
                        value={clampPercentForProgress(
                          usage.budgetConsumedPercent,
                        )}
                        status={provider.status}
                      />
                    ) : null}
                    {!hasUsableBudgetProgress ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-7 w-full justify-start px-0 text-xs"
                        aria-label={t("trayUsage.openProviderDetails", {
                          provider: provider.providerName,
                          defaultValue: "Open {{provider}} details",
                        })}
                        onClick={() => onOpenDetails(agentModuleId)}
                      >
                        <ExternalLink
                          className="h-3.5 w-3.5"
                          aria-hidden="true"
                        />
                        {t("trayUsage.openDetails", {
                          defaultValue: "Open details",
                        })}
                      </Button>
                    ) : null}
                  </>
                ) : (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 w-full justify-start px-0 text-xs"
                    aria-label={t("trayUsage.setProviderBudget", {
                      provider: provider.providerName,
                      defaultValue: "Set daily budget for {{provider}}",
                    })}
                    onClick={() => onOpenSettings(provider.providerId)}
                  >
                    <PlusCircle className="h-3.5 w-3.5" aria-hidden="true" />
                    {t("trayUsage.setDailyBudget", {
                      defaultValue: "Set daily budget",
                    })}
                  </Button>
                )}
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
