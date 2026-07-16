import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs } from "@/components/ui/tabs";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { TrayAgentTabs, trayAgentTabId } from "./TrayAgentTabs";
import { TrayApiSpendingSection } from "./TrayApiSpendingSection";
import { TraySubscriptionSection } from "./TraySubscriptionSection";
import { TrayUsageFooter } from "./TrayUsageFooter";
import { TrayUsageStatusBadge } from "./TrayUsageProgress";
import {
  formatUpdatedTime,
  type TrayProviderRow,
  type TrayUsageTranslate,
} from "./trayUsagePresentation";

export interface TrayUsagePopoverViewProps {
  snapshot: TrayUsageSnapshot | null;
  loading: boolean;
  selectedAgentId: "overview" | string;
  onSelectAgent: (agentModuleId: "overview" | string) => void;
  refreshing: boolean;
  onRefresh: () => void;
  onOpenDetails: (agentModuleId: string | null) => void;
  onOpenSettings: (providerId: string | null) => void;
  onQuit: () => void;
  focusRequestKey?: number;
  now?: Date;
}

export function TrayUsagePopoverView({
  snapshot,
  loading,
  selectedAgentId,
  onSelectAgent,
  refreshing,
  onRefresh,
  onOpenDetails,
  onOpenSettings,
  onQuit,
  focusRequestKey = 0,
  now = new Date(),
}: TrayUsagePopoverViewProps) {
  const { t, i18n } = useTranslation();
  const translate = t as TrayUsageTranslate;
  const locale = i18n.resolvedLanguage || i18n.language || "en-US";
  const agents = snapshot?.agents ?? [];
  const selectedAgent =
    selectedAgentId === "overview"
      ? null
      : (agents.find((agent) => agent.agentModuleId === selectedAgentId) ??
        null);
  const effectiveSelectedAgentId =
    selectedAgentId === "overview" || selectedAgent
      ? selectedAgentId
      : "overview";

  const rows = useMemo<TrayProviderRow[]>(() => {
    const visibleAgents = selectedAgent ? [selectedAgent] : agents;
    return visibleAgents.flatMap((agent) =>
      agent.providers.map((provider) => ({
        agentModuleId: agent.agentModuleId,
        agentName: agent.name,
        provider,
      })),
    );
  }, [agents, selectedAgent]);

  const subscriptionRows = rows.filter(
    (row) =>
      row.provider.billingKind === "subscription" &&
      row.provider.subscription != null,
  );
  const meteredRows = rows.filter(
    (row) =>
      row.provider.billingKind === "metered" && row.provider.metered != null,
  );
  const renderableRowCount = subscriptionRows.length + meteredRows.length;
  const status = selectedAgent?.status ?? snapshot?.status ?? "unknown";
  const overviewLabel = t("trayUsage.overview", { defaultValue: "Overview" });
  const title = selectedAgent?.name ?? overviewLabel;
  const updated = snapshot
    ? formatUpdatedTime(snapshot.lastSuccessAt, now, locale)
    : null;
  const freshnessText = updated
    ? t("trayUsage.updated", {
        time: updated,
        defaultValue: "Updated {{time}}",
      })
    : t("trayUsage.neverUpdated", {
        defaultValue: "No usable data yet",
      });
  const selectedDetailsAgent =
    effectiveSelectedAgentId === "overview" ? null : effectiveSelectedAgentId;
  const panelId = "tray-usage-agent-panel";
  const selectedTabId = trayAgentTabId(effectiveSelectedAgentId, agents);

  return (
    <main
      data-testid="tray-usage-popover"
      aria-busy={loading || refreshing}
      className="tray-popover-frame flex flex-col overflow-hidden"
    >
      <Tabs
        value={effectiveSelectedAgentId}
        onValueChange={onSelectAgent}
        className="flex min-h-0 flex-1 flex-col"
      >
        <TrayAgentTabs
          agents={agents}
          selectedAgentId={effectiveSelectedAgentId}
          overviewLabel={overviewLabel}
          navigationLabel={t("trayUsage.agentTabs", {
            defaultValue: "Agent tabs",
          })}
          panelId={panelId}
          focusRequestKey={focusRequestKey}
        />

        <div
          id={panelId}
          role="tabpanel"
          aria-labelledby={selectedTabId}
          className="flex min-h-0 flex-1 flex-col"
        >
          <header className="shrink-0 border-b border-border px-4 py-3">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h1
                  className="truncate text-[15px] font-semibold"
                  title={title}
                >
                  {title}
                </h1>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                  <span>{freshnessText}</span>
                  {snapshot?.stale ? (
                    <span className="font-medium text-foreground">
                      {t("trayUsage.stale", { defaultValue: "Stale" })}
                    </span>
                  ) : null}
                </div>
              </div>
              <TrayUsageStatusBadge status={status} t={translate} live />
            </div>
            {snapshot?.refreshError ? (
              <p
                role="alert"
                className="mt-2 flex items-start gap-1.5 text-[11px] text-muted-foreground"
              >
                <AlertTriangle
                  className="mt-px h-3.5 w-3.5 shrink-0 text-[#FFCC00]"
                  aria-hidden="true"
                />
                {t("trayUsage.refreshFailed", {
                  defaultValue: "Refresh failed; showing last successful data.",
                })}
              </p>
            ) : null}
          </header>

          <ScrollArea className="min-h-0 flex-1">
            <div className="space-y-4 px-4 py-3">
              {loading && !snapshot ? (
                <div role="status" className="space-y-3 py-5 text-center">
                  <p className="text-sm text-muted-foreground">
                    {t("trayUsage.loading", {
                      defaultValue: "Loading usage…",
                    })}
                  </p>
                  <div aria-hidden="true" className="space-y-2">
                    <div className="h-16 animate-pulse rounded-lg bg-muted" />
                    <div className="h-24 animate-pulse rounded-lg bg-muted" />
                  </div>
                </div>
              ) : (
                <>
                  <TraySubscriptionSection
                    rows={subscriptionRows}
                    now={now}
                    locale={locale}
                    showAgentName={effectiveSelectedAgentId === "overview"}
                  />
                  <TrayApiSpendingSection
                    rows={meteredRows}
                    locale={locale}
                    showAgentName={effectiveSelectedAgentId === "overview"}
                    onOpenDetails={onOpenDetails}
                    onOpenSettings={onOpenSettings}
                  />
                  {renderableRowCount === 0 ? (
                    <div className="py-10 text-center">
                      <p className="text-sm font-medium">
                        {t("trayUsage.empty", {
                          defaultValue: "No visible usage sources",
                        })}
                      </p>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {t("trayUsage.unknown", {
                          defaultValue: "Data unavailable",
                        })}
                      </p>
                    </div>
                  ) : null}
                </>
              )}
            </div>
          </ScrollArea>

          <TrayUsageFooter
            refreshing={refreshing}
            onOpenDetails={() => onOpenDetails(selectedDetailsAgent)}
            onRefresh={onRefresh}
            onOpenSettings={() => onOpenSettings(null)}
            onQuit={onQuit}
          />
        </div>
      </Tabs>
    </main>
  );
}
