import { useState } from "react";
import { Settings } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { UsageDashboardPage } from "@/components/usage-dashboard/UsageDashboardPage";
import { useTranslation } from "react-i18next";

export default function App() {
  const { t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <div className="flex h-screen min-h-0 flex-col bg-background text-foreground">
      <header className="flex shrink-0 items-center justify-between border-b border-border/70 px-6 py-3">
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
        <Button
          size="sm"
          variant="outline"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings className="mr-2 h-4 w-4" />
          {t("common.settings", { defaultValue: "Settings" })}
        </Button>
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
