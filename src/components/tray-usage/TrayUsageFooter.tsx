import { ExternalLink, Power, RefreshCw, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

/**
 * Popover action bar. "Open details" is the single primary action (filled),
 * while refresh / settings / quit are compact icon buttons — the hierarchy
 * makes the main-window entry point unmissable.
 */
export function TrayUsageFooter({
  refreshing,
  onOpenDetails,
  onRefresh,
  onOpenSettings,
  onQuit,
}: {
  refreshing: boolean;
  onOpenDetails: () => void;
  onRefresh: () => void;
  onOpenSettings: () => void;
  onQuit: () => void;
}) {
  const { t } = useTranslation();
  const refreshLabel = refreshing
    ? t("trayUsage.refreshing", { defaultValue: "Refreshing" })
    : t("trayUsage.refresh", { defaultValue: "Refresh" });

  const iconActions = [
    {
      key: "refresh",
      label: refreshLabel,
      icon: (
        <RefreshCw
          className={cn("h-4 w-4", refreshing && "animate-spin")}
          aria-hidden="true"
        />
      ),
      onClick: onRefresh,
      disabled: refreshing,
    },
    {
      key: "settings",
      label: t("trayUsage.settings", { defaultValue: "Settings" }),
      icon: <Settings className="h-4 w-4" aria-hidden="true" />,
      onClick: onOpenSettings,
      disabled: false,
    },
    {
      key: "quit",
      label: t("trayUsage.quit", { defaultValue: "Quit" }),
      icon: <Power className="h-4 w-4" aria-hidden="true" />,
      onClick: onQuit,
      disabled: false,
    },
  ];

  return (
    <footer className="tray-usage-footer flex shrink-0 items-center gap-1.5 border-t border-border/60 bg-popover px-2.5 py-2">
      <button
        type="button"
        onClick={onOpenDetails}
        aria-label={t("trayUsage.openDetails", {
          defaultValue: "Open details",
        })}
        className="tray-usage-footer-button flex h-8 flex-1 items-center justify-center gap-1.5 rounded-md bg-primary text-xs font-medium text-primary-foreground shadow-xs transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
      >
        <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
        <span className="tray-usage-footer-label">
          {t("trayUsage.openDetails", { defaultValue: "Open details" })}
        </span>
      </button>
      {iconActions.map((action) => (
        <button
          key={action.key}
          type="button"
          onClick={action.onClick}
          disabled={action.disabled}
          aria-label={action.label}
          title={action.label}
          className="tray-usage-footer-button grid h-8 w-8 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50"
        >
          {action.icon}
          <span className="sr-only">{action.label}</span>
        </button>
      ))}
    </footer>
  );
}
