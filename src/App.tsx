import { useState } from "react";
import { Maximize2, Minus, Settings, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { UsageDashboardPage } from "@/components/usage-dashboard/UsageDashboardPage";
import { useTranslation } from "react-i18next";
import { useSettingsQuery } from "@/lib/query";
import { isMac } from "@/lib/platform";

export default function App() {
  const { t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { data: settings } = useSettingsQuery();
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
        className="flex shrink-0 items-center justify-between border-b border-border/70 px-6 py-3"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
      >
        <div>
          <h1 className="text-lg font-semibold">
            {t("app.title", { defaultValue: "LLM Usage Bar" })}
          </h1>
          <p className="text-xs text-muted-foreground">
            {t("usageDashboard.description", {
              defaultValue:
                "Provider-aware tokens, costs and subscription quota",
            })}
          </p>
        </div>
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
        <UsageDashboardPage />
      </main>

      <SettingsPage
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        defaultTab="usage"
      />
    </div>
  );
}
