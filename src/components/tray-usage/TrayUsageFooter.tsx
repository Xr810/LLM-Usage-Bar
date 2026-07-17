import { ExternalLink, Power, RefreshCw, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

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

  const actions = [
    {
      key: "details",
      label: t("trayUsage.openDetails", { defaultValue: "Open details" }),
      icon: <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />,
      onClick: onOpenDetails,
      disabled: false,
    },
    {
      key: "refresh",
      label: refreshLabel,
      icon: (
        <RefreshCw
          className={cn("h-3.5 w-3.5", refreshing && "animate-spin")}
          aria-hidden="true"
        />
      ),
      onClick: onRefresh,
      disabled: refreshing,
    },
    {
      key: "settings",
      label: t("trayUsage.settings", { defaultValue: "Settings" }),
      icon: <Settings className="h-3.5 w-3.5" aria-hidden="true" />,
      onClick: onOpenSettings,
      disabled: false,
    },
    {
      key: "quit",
      label: t("trayUsage.quit", { defaultValue: "Quit" }),
      icon: <Power className="h-3.5 w-3.5" aria-hidden="true" />,
      onClick: onQuit,
      disabled: false,
    },
  ];

  return (
    <footer className="tray-usage-footer grid shrink-0 grid-cols-4 gap-0.5 border-t border-border/60 bg-popover px-1.5 py-1.5">
      {actions.map((action) => (
        <button
          key={action.key}
          type="button"
          onClick={action.onClick}
          disabled={action.disabled}
          aria-label={action.label}
          className="tray-usage-footer-button flex min-w-0 flex-col items-center justify-center gap-0.5 rounded-md px-1 py-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50"
        >
          {action.icon}
          <span className="tray-usage-footer-label text-[10px] leading-tight">
            {action.label}
          </span>
        </button>
      ))}
    </footer>
  );
}
