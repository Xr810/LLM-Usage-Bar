import { ExternalLink, Power, RefreshCw, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
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

  return (
    <footer className="tray-usage-footer grid shrink-0 grid-cols-2 gap-1 border-t border-border bg-popover px-2 py-2">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="tray-usage-footer-button min-w-0 px-2"
        onClick={onOpenDetails}
        aria-label={t("trayUsage.openDetails", {
          defaultValue: "Open details",
        })}
      >
        <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
        <span className="tray-usage-footer-label whitespace-normal text-center leading-tight">
          {t("trayUsage.openDetails", { defaultValue: "Open details" })}
        </span>
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="tray-usage-footer-button min-w-0 px-2"
        onClick={onRefresh}
        disabled={refreshing}
        aria-label={refreshLabel}
      >
        <RefreshCw
          className={cn("h-3.5 w-3.5", refreshing && "animate-spin")}
          aria-hidden="true"
        />
        <span className="tray-usage-footer-label whitespace-normal text-center leading-tight">
          {refreshLabel}
        </span>
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="tray-usage-footer-button min-w-0 px-2"
        onClick={onOpenSettings}
        aria-label={t("trayUsage.settings", { defaultValue: "Settings" })}
      >
        <Settings className="h-3.5 w-3.5" aria-hidden="true" />
        <span className="tray-usage-footer-label whitespace-normal text-center leading-tight">
          {t("trayUsage.settings", { defaultValue: "Settings" })}
        </span>
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="tray-usage-footer-button min-w-0 px-2"
        onClick={onQuit}
        aria-label={t("trayUsage.quit", { defaultValue: "Quit" })}
      >
        <Power className="h-3.5 w-3.5" aria-hidden="true" />
        <span className="tray-usage-footer-label whitespace-normal text-center leading-tight">
          {t("trayUsage.quit", { defaultValue: "Quit" })}
        </span>
      </Button>
    </footer>
  );
}
