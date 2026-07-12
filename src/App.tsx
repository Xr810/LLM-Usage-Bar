import { useState } from "react";
import { Maximize2, Minus, Settings, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { UsageDashboardPage } from "@/components/usage-dashboard/UsageDashboardPage";
import { DashboardModuleSwitcher } from "@/components/usage-dashboard/DashboardModuleSwitcher";
import { useDashboardModuleSelection } from "@/components/usage-dashboard/useDashboardModuleSelection";
import { useTranslation } from "react-i18next";
import { useSettingsQuery } from "@/lib/query";
import { useDashboardModules } from "@/lib/query/usageDashboard";
import { isMac } from "@/lib/platform";

export default function App() {
  const { t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { data: settings } = useSettingsQuery();
  const modules = useDashboardModules();
  const { selectedModule, selectModule } = useDashboardModuleSelection(
    modules.data ?? [],
  );
  const useAppWindowControls = settings?.useAppWindowControls ?? false;

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
    <div className="flex h-screen min-h-0 flex-col bg-background text-foreground">
      {isMac() ? (
        <div
          aria-hidden="true"
          className="h-7 shrink-0"
          data-tauri-drag-region
          style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        />
      ) : null}
      <header
        className="grid shrink-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-4 border-b border-border/70 px-6 py-3"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
      >
        <h1 className="text-lg font-semibold">
          {t("app.title", { defaultValue: "LLM Usage Bar" })}
        </h1>
        <DashboardModuleSwitcher
          modules={modules.data ?? []}
          selectedModuleId={selectedModule?.id ?? ""}
          onSelect={selectModule}
        />
        <div
          className="flex items-center gap-1"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <Button
            size="sm"
            variant="outline"
            onClick={() => setSettingsOpen(true)}
          >
            <Settings className="mr-2 h-4 w-4" />
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

      <main className="min-h-0 flex-1 overflow-y-auto px-6 pt-5">
        {modules.error ? (
          <Alert variant="destructive">
            <AlertDescription className="flex items-center justify-between gap-3">
              <span>
                {modules.error instanceof Error
                  ? modules.error.message
                  : String(modules.error)}
              </span>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void modules.refetch()}
              >
                {t("common.retry", { defaultValue: "Retry" })}
              </Button>
            </AlertDescription>
          </Alert>
        ) : modules.isLoading ? (
          <div>{t("common.loading", { defaultValue: "Loading" })}</div>
        ) : selectedModule ? (
          <UsageDashboardPage
            selectedModule={selectedModule}
            onOpenSettings={() => setSettingsOpen(true)}
          />
        ) : (
          <div className="rounded-lg border border-dashed p-8 text-center">
            <p className="text-sm text-muted-foreground">
              {t("dashboardModules.noneVisible", {
                defaultValue: "No visible usage modules",
              })}
            </p>
            <Button
              className="mt-3"
              size="sm"
              onClick={() => setSettingsOpen(true)}
            >
              {t("dashboardModules.manage", {
                defaultValue: "Manage usage modules",
              })}
            </Button>
          </div>
        )}
      </main>

      <SettingsPage
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        defaultTab="modules"
      />
    </div>
  );
}
