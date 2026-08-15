import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import { SegmentedControl } from "@/components/common/SegmentedControl";
import type { ModelUsageDashboardView } from "@/types/usageDashboard";
import {
  costStatusLabel,
  costText,
  modelLabel,
  productGroupLabel,
  rowCostStatus,
  sharePercent,
} from "./breakdownPresentation";
import { UsageBreakdownHeader, UsageBreakdownRow } from "./UsageBreakdownRow";
import { BreakdownSummary } from "./BreakdownSummary";
import { BreakdownEmpty } from "./BreakdownEmpty";
import { dashboardProviderIcon } from "./usagePresentation";

type ModelGrouping = "product" | "model";

interface ModelUsagePageProps {
  dashboard: ModelUsageDashboardView;
  rangeLabel: string;
  rangeControls?: ReactNode;
}

/**
 * The "By model" classification.
 *
 * Two readings of the same range, chosen with one control:
 * - **By plan** rolls usage up to the product a subscription or API account
 *   belongs to ("how much did Claude Pro cost me"), then drills into the
 *   models served inside it;
 * - **By model** merges each model across every account that served it
 *   ("how much Opus 5 did I use, wherever it ran").
 */
export function ModelUsagePage({
  dashboard,
  rangeLabel,
  rangeControls,
}: ModelUsagePageProps) {
  const { t } = useTranslation();
  const [grouping, setGrouping] = useState<ModelGrouping>("product");

  const providerNameById = useMemo(() => {
    const names = new Map<string, string>();
    for (const group of dashboard.productGroups) {
      group.providerIds.forEach((providerId, index) => {
        const name = group.providerNames[index];
        if (name) names.set(providerId, name);
      });
    }
    return names;
  }, [dashboard.productGroups]);

  const groupingOptions = [
    {
      value: "product" as const,
      label: t("usageBreakdown.byPlan", { defaultValue: "By plan" }),
    },
    {
      value: "model" as const,
      label: t("usageBreakdown.byModel", { defaultValue: "By model" }),
    },
  ];

  const listLabel =
    grouping === "product"
      ? t("usageBreakdown.plan", { defaultValue: "Plan" })
      : t("usageBreakdown.model", { defaultValue: "Model" });

  const empty =
    dashboard.totalEventCount === 0 || dashboard.productGroups.length === 0;

  return (
    <div className="space-y-4">
      <BreakdownSummary
        totalTokens={dashboard.totalTokens}
        totalEventCount={dashboard.totalEventCount}
        totalCostUsd={dashboard.totalCostUsd}
        rangeLabel={rangeLabel}
        rangeControls={rangeControls}
        countLabel={t("usageBreakdown.modelCount", {
          count: dashboard.models.length,
          defaultValue: "{{count}} models",
        })}
      />

      {dashboard.warnings.map((warning) => (
        <p key={warning} className="text-xs text-muted-foreground">
          {warning}
        </p>
      ))}

      {empty ? (
        <BreakdownEmpty
          text={t("usageBreakdown.modelEmpty", {
            defaultValue: "No model usage was recorded in this range.",
          })}
        />
      ) : (
        <Card className="overflow-hidden">
          <div className="flex flex-wrap items-center justify-between gap-3 px-3 py-3">
            <h3 className="text-sm font-semibold tracking-tight">
              {t("usageBreakdown.modelBreakdown", {
                defaultValue: "Model breakdown",
              })}
            </h3>
            <SegmentedControl
              options={groupingOptions}
              value={grouping}
              onChange={setGrouping}
            />
          </div>

          <UsageBreakdownHeader
            label={listLabel}
            shareLabel={t("usageBreakdown.share", { defaultValue: "Share" })}
            tokensLabel={t("usageDashboard.tokens", { defaultValue: "Tokens" })}
            requestsLabel={t("usageDashboard.requests", {
              defaultValue: "Requests",
            })}
          />

          <div className="divide-y divide-border/40">
            {grouping === "product"
              ? dashboard.productGroups.map((group) => {
                  const label = productGroupLabel(
                    group.productGroupId,
                    group.providerNames,
                    t,
                  );
                  const { icon, iconColor } = dashboardProviderIcon({
                    systemPresetKey: null,
                    productGroupId: group.productGroupId,
                    name: label,
                  });
                  // A single account whose name already is the plan name would
                  // just repeat the title, so the subtitle is dropped.
                  const accountsText = group.providerNames.join(" · ");
                  return (
                    <UsageBreakdownRow
                      key={group.productGroupId}
                      data-testid={`model-group-${group.productGroupId}`}
                      title={label}
                      subtitle={
                        accountsText === label ? undefined : accountsText
                      }
                      leading={
                        <ProviderIcon
                          icon={icon}
                          color={iconColor}
                          name={label}
                          size={22}
                          className="shrink-0 rounded-[6px] border border-border/50"
                        />
                      }
                      badges={
                        <Badge variant="secondary" className="shrink-0">
                          {group.billingKind === "subscription"
                            ? t("usageDashboard.subscription", {
                                defaultValue: "Subscription",
                              })
                            : t("usageDashboard.metered", {
                                defaultValue: "Metered",
                              })}
                        </Badge>
                      }
                      share={sharePercent(
                        group.totalTokens,
                        dashboard.totalTokens,
                      )}
                      tokens={group.totalTokens}
                      requests={group.eventCount}
                      costText={costText(group.totalCostUsd, t)}
                      costMuted={group.totalCostUsd == null}
                    >
                      {group.models.map((model) => (
                        <UsageBreakdownRow
                          key={`${model.providerId}:${model.model}`}
                          variant="nested"
                          title={modelLabel(model.model, t)}
                          subtitle={
                            group.providerIds.length > 1
                              ? model.providerName
                              : undefined
                          }
                          share={sharePercent(
                            model.totalTokens,
                            group.totalTokens,
                          )}
                          tokens={model.totalTokens}
                          requests={model.eventCount}
                          costText={costText(model.totalCostUsd, t)}
                          costMuted={model.totalCostUsd == null}
                        />
                      ))}
                    </UsageBreakdownRow>
                  );
                })
              : dashboard.models.map((model) => {
                  const accounts = model.providerIds
                    .map((providerId) => providerNameById.get(providerId))
                    .filter((name): name is string => Boolean(name));
                  const status = rowCostStatus(
                    model.totalCostUsd,
                    model.costSourceCounts,
                  );
                  return (
                    <UsageBreakdownRow
                      key={model.model}
                      data-testid={`model-total-${model.model}`}
                      title={
                        <span className="font-mono text-[13px]">
                          {modelLabel(model.model, t)}
                        </span>
                      }
                      subtitle={accounts.join(" · ")}
                      badges={
                        status === "complete" ? null : (
                          <Badge variant="outline" className="shrink-0">
                            {costStatusLabel(status, t)}
                          </Badge>
                        )
                      }
                      share={sharePercent(
                        model.totalTokens,
                        dashboard.totalTokens,
                      )}
                      tokens={model.totalTokens}
                      requests={model.eventCount}
                      costText={costText(model.totalCostUsd, t)}
                      costMuted={model.totalCostUsd == null}
                    />
                  );
                })}
          </div>
        </Card>
      )}
    </div>
  );
}
