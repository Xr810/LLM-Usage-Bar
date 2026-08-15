import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { ProviderIcon } from "@/components/ProviderIcon";
import type {
  AgentUsageBreakdownView,
  AgentUsageRow,
} from "@/types/usageDashboard";
import {
  agentLabel,
  costText,
  modelLabel,
  productGroupLabel,
  sharePercent,
} from "./breakdownPresentation";
import { UsageBreakdownHeader, UsageBreakdownRow } from "./UsageBreakdownRow";
import { BreakdownSummary } from "./BreakdownSummary";
import { BreakdownEmpty } from "./BreakdownEmpty";
import { dashboardProviderIcon } from "./usagePresentation";

interface AgentBreakdownPageProps {
  breakdown: AgentUsageBreakdownView;
  rangeLabel: string;
  rangeControls?: ReactNode;
}

/**
 * The "By agent" classification: what each Agent spent in the range,
 * drilling into the Provider accounts and models it used.
 *
 * Agents with no usage in the range are absent by design — this is a
 * breakdown of what happened, not a roster of what is configured.
 */
export function AgentBreakdownPage({
  breakdown,
  rangeLabel,
  rangeControls,
}: AgentBreakdownPageProps) {
  const { t } = useTranslation();
  const empty =
    breakdown.agents.length === 0 || breakdown.totalEventCount === 0;

  return (
    <div className="space-y-4">
      <BreakdownSummary
        totalTokens={breakdown.totalTokens}
        totalEventCount={breakdown.totalEventCount}
        totalCostUsd={breakdown.totalCostUsd}
        rangeLabel={rangeLabel}
        rangeControls={rangeControls}
        countLabel={t("usageBreakdown.agentCount", {
          count: breakdown.agents.length,
          defaultValue: "{{count}} agents",
        })}
      />

      {breakdown.warnings.map((warning) => (
        <p key={warning} className="text-xs text-muted-foreground">
          {warning}
        </p>
      ))}

      {empty ? (
        <BreakdownEmpty
          text={t("usageBreakdown.agentEmpty", {
            defaultValue: "No Agent usage was recorded in this range.",
          })}
        />
      ) : (
        <Card className="overflow-hidden">
          <div className="px-3 py-3">
            <h3 className="text-sm font-semibold tracking-tight">
              {t("usageBreakdown.agentBreakdown", {
                defaultValue: "Agent breakdown",
              })}
            </h3>
          </div>

          <UsageBreakdownHeader
            label={t("usageBreakdown.agent", { defaultValue: "Agent" })}
            shareLabel={t("usageBreakdown.share", { defaultValue: "Share" })}
            tokensLabel={t("usageDashboard.tokens", { defaultValue: "Tokens" })}
            requestsLabel={t("usageDashboard.requests", {
              defaultValue: "Requests",
            })}
          />

          <div className="divide-y divide-border/40">
            {breakdown.agents.map((agent) => (
              <AgentRow
                key={agent.agentModuleId ?? "__unassigned__"}
                agent={agent}
                rangeTotalTokens={breakdown.totalTokens}
              />
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}

function AgentRow({
  agent,
  rangeTotalTokens,
}: {
  agent: AgentUsageRow;
  rangeTotalTokens: number;
}) {
  const { t } = useTranslation();
  const label = agentLabel(agent.agentModuleId, agent.agentName, t);
  const badges: ReactNode[] = [];
  if (agent.agentModuleId == null) {
    badges.push(
      <Badge key="unassigned" variant="outline" className="shrink-0">
        {t("usageBreakdown.unassignedAgent", { defaultValue: "Unassigned" })}
      </Badge>,
    );
  } else if (agent.archived) {
    badges.push(
      <Badge key="archived" variant="secondary" className="shrink-0">
        {t("usageBreakdown.archivedAgent", { defaultValue: "Archived" })}
      </Badge>,
    );
  }

  return (
    <UsageBreakdownRow
      data-testid={`agent-row-${agent.agentModuleId ?? "unassigned"}`}
      title={label}
      subtitle={t("usageBreakdown.agentProviderCount", {
        count: agent.providers.length,
        defaultValue: "{{count}} accounts",
      })}
      badges={badges.length ? <>{badges}</> : null}
      share={sharePercent(agent.totalTokens, rangeTotalTokens)}
      tokens={agent.totalTokens}
      requests={agent.eventCount}
      costText={costText(agent.totalCostUsd, t)}
      costMuted={agent.totalCostUsd == null}
    >
      <NestedSection
        title={t("usageBreakdown.accounts", { defaultValue: "Accounts" })}
      >
        {agent.providers.map((provider) => {
          const providerLabel =
            provider.providerName.trim() ||
            productGroupLabel(provider.productGroupId, [], t);
          const { icon, iconColor } = dashboardProviderIcon({
            systemPresetKey: null,
            productGroupId: provider.productGroupId,
            name: providerLabel,
          });
          return (
            <UsageBreakdownRow
              key={provider.providerId}
              variant="nested"
              title={
                <span className="font-sans text-[12px] font-medium">
                  {providerLabel}
                </span>
              }
              leading={
                <ProviderIcon
                  icon={icon}
                  color={iconColor}
                  name={providerLabel}
                  size={18}
                  className="shrink-0 rounded-[5px] border border-border/50"
                />
              }
              share={sharePercent(provider.totalTokens, agent.totalTokens)}
              tokens={provider.totalTokens}
              requests={provider.eventCount}
              costText={costText(provider.totalCostUsd, t)}
              costMuted={provider.totalCostUsd == null}
            />
          );
        })}
      </NestedSection>

      <NestedSection
        title={t("usageBreakdown.models", { defaultValue: "Models" })}
      >
        {agent.models.map((model) => (
          <UsageBreakdownRow
            key={model.model}
            variant="nested"
            title={modelLabel(model.model, t)}
            share={sharePercent(model.totalTokens, agent.totalTokens)}
            tokens={model.totalTokens}
            requests={model.eventCount}
            costText={costText(model.totalCostUsd, t)}
            costMuted={model.totalCostUsd == null}
          />
        ))}
      </NestedSection>
    </UsageBreakdownRow>
  );
}

function NestedSection({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="border-b border-border/30 py-1 last:border-b-0">
      <div className="px-3 pt-1.5 pb-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/80">
        {title}
      </div>
      {children}
    </div>
  );
}
