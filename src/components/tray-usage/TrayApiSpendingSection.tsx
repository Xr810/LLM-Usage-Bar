import { ExternalLink, PlusCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  clampPercentForProgress,
  costQualityLabel,
  formatUsd,
  hasUsableBudget,
  hasUsablePercent,
  hasUsableUsd,
  providerIconName,
  type TrayProviderRow,
} from "./trayUsagePresentation";
import { TrayUsageProgress, TrayUsageStatusBadge } from "./TrayUsageProgress";
import { TrayProviderRecentUsage } from "./TrayProviderRecentUsage";

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
        className="mb-1.5 px-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground"
      >
        {t("trayUsage.apiSpending", { defaultValue: "API spending" })}
      </h2>
      <div className="space-y-2">
        {rows.map(({ agentModuleId, agentName, provider }) => {
          if (!provider.metered) return null;
          const usage = provider.metered;
          const todayCost =
            usage.costQuality === "unavailable" ? null : usage.todayCostUsd;
          const hasBudget = hasUsableBudget(usage.dailyBudgetUsd);
          const hasUsableBudgetProgress =
            hasBudget &&
            usage.costQuality !== "unavailable" &&
            hasUsableUsd(todayCost) &&
            hasUsablePercent(usage.budgetConsumedPercent);
          const quality = costQualityLabel(usage.costQuality, t);
          const dailyBudgetLabel = t("trayUsage.dailyBudget", {
            defaultValue: "Daily budget",
          });
          const budgetProgressLabel = showAgentName
            ? t("trayUsage.providerAllowanceForAgent", {
                allowance: dailyBudgetLabel,
                provider: provider.providerName,
                agent: agentName,
                defaultValue: "{{allowance}} for {{provider}} · {{agent}}",
              })
            : t("trayUsage.providerAllowance", {
                allowance: dailyBudgetLabel,
                provider: provider.providerName,
                defaultValue: "{{allowance}} for {{provider}}",
              });

          return (
            <article
              key={`${agentModuleId}:${provider.providerId}`}
              className="tray-usage-provider-row space-y-2.5 rounded-xl border border-border/60 bg-card"
            >
              <div className="flex min-w-0 items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <ProviderIcon
                    icon={providerIconName(provider.systemPresetKey)}
                    name={provider.providerName}
                    size={22}
                    className="rounded-md"
                  />
                  <div className="min-w-0">
                    <h3
                      className="truncate text-[13px] font-semibold leading-tight"
                      title={provider.providerName}
                    >
                      {provider.providerName}
                    </h3>
                    {showAgentName ? (
                      <p className="truncate text-[11px] leading-tight text-muted-foreground">
                        {agentName}
                      </p>
                    ) : null}
                  </div>
                </div>
                <TrayUsageStatusBadge status={provider.status} t={t} />
              </div>

              <dl className="grid grid-cols-2 gap-x-3 rounded-lg bg-muted/25 px-2.5 py-2 dark:bg-muted/15">
                <div className="min-w-0">
                  <dt className="truncate text-[11px] text-muted-foreground">
                    {t("trayUsage.today", { defaultValue: "Today" })}
                  </dt>
                  <dd className="mt-0.5 truncate text-[13px] font-semibold tabular-nums">
                    {formatUsd(todayCost, locale)}
                  </dd>
                </div>
                <div className="min-w-0">
                  <dt className="truncate text-[11px] text-muted-foreground">
                    {t("trayUsage.dailyBudget", {
                      defaultValue: "Daily budget",
                    })}
                  </dt>
                  <dd className="mt-0.5 truncate text-[13px] font-semibold tabular-nums">
                    {formatUsd(usage.dailyBudgetUsd, locale)}
                  </dd>
                </div>
              </dl>

              <div className="space-y-1">
                <div className="flex items-baseline justify-between gap-3 text-xs">
                  <span className="text-muted-foreground">
                    {dailyBudgetLabel}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    {quality}
                  </span>
                </div>

                {hasBudget ? (
                  <>
                    <div className="text-[13px] font-semibold tabular-nums">
                      {t("trayUsage.ofBudget", {
                        cost: formatUsd(todayCost, locale),
                        budget: formatUsd(usage.dailyBudgetUsd, locale),
                        defaultValue: "{{cost}} of {{budget}}",
                      })}
                    </div>
                    {hasUsableBudgetProgress ? (
                      <TrayUsageProgress
                        label={budgetProgressLabel}
                        value={clampPercentForProgress(
                          usage.budgetConsumedPercent,
                        )}
                        status={provider.status}
                      />
                    ) : null}
                    {!hasUsableBudgetProgress ? (
                      <button
                        type="button"
                        className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                        aria-label={t("trayUsage.openProviderDetails", {
                          provider: provider.providerName,
                          defaultValue: "Open {{provider}} details",
                        })}
                        onClick={() => onOpenDetails(agentModuleId)}
                      >
                        <ExternalLink className="h-3 w-3" aria-hidden="true" />
                        {t("trayUsage.openDetails", {
                          defaultValue: "Open details",
                        })}
                      </button>
                    ) : null}
                  </>
                ) : (
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                    aria-label={t("trayUsage.setProviderBudget", {
                      provider: provider.providerName,
                      defaultValue: "Set daily budget for {{provider}}",
                    })}
                    onClick={() => onOpenSettings(provider.providerId)}
                  >
                    <PlusCircle className="h-3 w-3" aria-hidden="true" />
                    {t("trayUsage.setDailyBudget", {
                      defaultValue: "Set daily budget",
                    })}
                  </button>
                )}
              </div>

              <TrayProviderRecentUsage
                usage={provider.recentUsage}
                locale={locale}
              />
            </article>
          );
        })}
      </div>
    </section>
  );
}
