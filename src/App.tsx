import { useCallback, useState } from "react";
import { ChartColumn, Maximize2, Minus, Settings, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { UsageDashboardPage } from "@/components/usage-dashboard/UsageDashboardPage";
import { useTranslation } from "react-i18next";
import { useSettingsQuery } from "@/lib/query";
import { isMac } from "@/lib/platform";
import { useMainWindowNavigation } from "@/hooks/useMainWindowNavigation";

export default function App() {
  const { t } = useTranslation();
  const [settingsTarget, setSettingsTarget] = useState<{
    open: boolean;
    tab: string;
    providerId: string | null;
  }>({ open: false, tab: "general", providerId: null });
  const { data: settings } = useSettingsQuery();
  const useAppWindowControls = settings?.useAppWindowControls ?? false;

  const openGeneralSettings = useCallback(() => {
    setSettingsTarget({ open: true, tab: "general", providerId: null });
  }, []);

  const openProviderSettings = useCallback((providerId: string | null) => {
    setSettingsTarget({ open: true, tab: "providers", providerId });
  }, []);

  const openProviderConfiguration = useCallback(() => {
    openProviderSettings(null);
  }, [openProviderSettings]);

  const openUsage = useCallback(() => {
    setSettingsTarget((current) => ({
      ...current,
      open: false,
      providerId: null,
    }));
  }, []);

  const setSettingsOpen = useCallback((open: boolean) => {
    setSettingsTarget((current) => ({ ...current, open }));
  }, []);

  const clearProviderTarget = useCallback(() => {
    setSettingsTarget((current) =>
      current.providerId === null ? current : { ...current, providerId: null },
    );
  }, []);

  useMainWindowNavigation({
    openUsage,
    openProviderSettings,
    openGeneralSettings,
  });

  const runWindowAction = async (
    action: "minimize" | "toggleMaximize" | "close",
  ) => {
    try {
      await getCurrentWindow()[action]();
    } catch (error) {
      console.error(`[App] Failed to ${action} window`, error);
    }
  };

  return (
    <div
      className="app-window-shell flex h-screen min-h-0 flex-col text-foreground"
      data-settings-open={settingsTarget.open || undefined}
    >
      {isMac() ? (
        <div
          aria-hidden="true"
          className="app-titlebar-spacer h-7 shrink-0"
          data-tauri-drag-region
          style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        />
      ) : null}
      <header
        className="app-window-header grid shrink-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-4 border-b border-border/60 px-5 py-2.5"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
      >
        <div className="flex items-center gap-2.5">
          <div
            aria-hidden="true"
            className="grid h-6 w-6 place-items-center rounded-md bg-primary text-primary-foreground shadow-xs"
          >
            <ChartColumn className="h-3.5 w-3.5" />
          </div>
          <h1 className="text-[15px] font-semibold tracking-tight">
            {t("app.title", { defaultValue: "LLM Usage Bar" })}
          </h1>
        </div>
        <div aria-hidden="true" />
        <div
          className="flex items-center gap-1"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <Button size="sm" variant="ghost" onClick={openGeneralSettings}>
            <Settings className="mr-1.5 h-3.5 w-3.5" />
            {t("common.settings", { defaultValue: "Settings" })}
          </Button>
          {useAppWindowControls ? (
            <>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                aria-label={t("header.windowMinimize", {
                  defaultValue: "Minimize window",
                })}
                onClick={() => void runWindowAction("minimize")}
              >
                <Minus className="h-4 w-4" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                aria-label={t("header.windowMaximize", {
                  defaultValue: "Maximize window",
                })}
                onClick={() => void runWindowAction("toggleMaximize")}
              >
                <Maximize2 className="h-4 w-4" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8 hover:bg-red-500/15 hover:text-red-500"
                aria-label={t("header.windowClose", {
                  defaultValue: "Close window",
                })}
                onClick={() => void runWindowAction("close")}
              >
                <X className="h-4 w-4" />
              </Button>
            </>
          ) : null}
        </div>
      </header>

      <main className="app-window-content min-h-0 flex-1 overflow-y-auto px-5 pt-5">
        <UsageDashboardPage onOpenSettings={openProviderConfiguration} />
      </main>

      <SettingsPage
        open={settingsTarget.open}
        onOpenChange={setSettingsOpen}
        defaultTab={settingsTarget.tab}
        defaultProviderId={settingsTarget.providerId ?? undefined}
        onProviderTargetHandled={clearProviderTarget}
      />
    </div>
  );
}
