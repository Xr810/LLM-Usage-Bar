import { EyeOff, Power } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { ToggleRow } from "@/components/ui/toggle-row";

export function StartupSettings() {
  const { t } = useTranslation();
  const { settings, isLoading, isSaving, updateSettings, autoSaveSettings } =
    useSettings();

  const save = (updates: Parameters<typeof updateSettings>[0]) => {
    updateSettings(updates);
    void autoSaveSettings(updates);
  };

  return (
    <section className="space-y-4 py-4">
      <div>
        <h3 className="text-sm font-medium">
          {t("settings.startup", { defaultValue: "Startup" })}
        </h3>
        <p className="mt-1 text-xs text-muted-foreground">
          {t("settings.startupDescription", {
            defaultValue: "Control how LLM Usage Bar starts after you log in.",
          })}
        </p>
      </div>

      <div className="space-y-3">
        <ToggleRow
          icon={<Power className="h-4 w-4 text-orange-500" />}
          title={t("settings.launchOnStartup")}
          description={t("settings.launchOnStartupDescription")}
          checked={!!settings?.launchOnStartup}
          onCheckedChange={(value) => save({ launchOnStartup: value })}
          disabled={isLoading || isSaving || !settings}
        />

        {settings?.launchOnStartup ? (
          <ToggleRow
            icon={<EyeOff className="h-4 w-4 text-green-500" />}
            title={t("settings.silentStartup")}
            description={t("settings.silentStartupDescription")}
            checked={!!settings.silentStartup}
            onCheckedChange={(value) => save({ silentStartup: value })}
            disabled={isSaving}
          />
        ) : null}
      </div>
    </section>
  );
}
