import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { TrayPanelV5 } from "@/components/lub/TrayPanelV5";
import { mapSnapshotToPanel } from "@/components/lub/mapSnapshotToPanel";
import type { TrayUsageSnapshot } from "@/types/trayUsage";

export interface TrayUsagePopoverViewProps {
  snapshot: TrayUsageSnapshot | null;
  loading: boolean;
  refreshing: boolean;
  onRefresh: () => void;
  onOpenDetails: () => void;
  onOpenSettings: (providerId: string | null) => void;
  onQuit: () => void;
  /** Deprecated compatibility props; Provider-only UI does not render Agent tabs. */
  selectedAgentId?: "overview" | string;
  onSelectAgent?: (agentModuleId: "overview" | string) => void;
  focusRequestKey?: number;
  now?: Date;
  /** C2 `uiFlags.upsellEntry`. Off until get_license_status says otherwise. */
  upsellEntry?: boolean;
}

export function TrayUsagePopoverView({
  snapshot,
  loading,
  refreshing,
  onRefresh,
  onOpenDetails,
  onOpenSettings,
  onQuit,
  upsellEntry = false,
}: TrayUsagePopoverViewProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const model = useMemo(
    () => (snapshot ? mapSnapshotToPanel(snapshot, onOpenSettings) : null),
    [onOpenSettings, snapshot],
  );

  return (
    <main
      data-testid="tray-usage-popover"
      aria-busy={loading || refreshing}
      className="tray-popover-frame"
    >
      {loading && !snapshot ? (
        <section className="lub-panel" role="status">
          <p className="lub-loading">
            {t("trayUsage.loading", { defaultValue: "Loading usage…" })}
          </p>
        </section>
      ) : model ? (
        <TrayPanelV5
          model={model}
          locale={locale}
          refreshing={refreshing}
          showUtilityActions
          onOpenTrends={onOpenDetails}
          onOpenSettings={() => onOpenSettings(null)}
          onRefresh={onRefresh}
          onQuit={onQuit}
          build={model.buildType}
          upsellEntry={upsellEntry}
          refreshError={
            snapshot?.refreshError
              ? t("trayUsage.refreshFailed", {
                  defaultValue: "Refresh failed; showing last successful data.",
                })
              : null
          }
        />
      ) : (
        <section className="lub-panel">
          <p className="lub-empty">
            {t("trayUsage.empty", {
              defaultValue: "No visible Provider accounts",
            })}
          </p>
        </section>
      )}
    </main>
  );
}
