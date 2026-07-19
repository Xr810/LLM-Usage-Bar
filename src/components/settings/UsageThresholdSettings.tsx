import { useEffect, useState } from "react";
import { Gauge } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { useSettings } from "@/hooks/useSettings";

const DEFAULT_WARNING = 50;
const DEFAULT_CRITICAL = 20;

export function UsageThresholdSettings() {
  const { t } = useTranslation();
  const { settings, isLoading, isSaving, updateSettings, autoSaveSettings } =
    useSettings();
  const [warning, setWarning] = useState(DEFAULT_WARNING);
  const [critical, setCritical] = useState(DEFAULT_CRITICAL);
  const [saveError, setSaveError] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setWarning(settings.usageWarningRemainingPercent ?? DEFAULT_WARNING);
    setCritical(settings.usageCriticalRemainingPercent ?? DEFAULT_CRITICAL);
  }, [
    settings?.usageWarningRemainingPercent,
    settings?.usageCriticalRemainingPercent,
  ]);

  const save = async () => {
    const updates = {
      usageWarningRemainingPercent: warning,
      usageCriticalRemainingPercent: critical,
    };
    setSaveError(false);
    updateSettings(updates);
    try {
      const result = await autoSaveSettings(updates);
      setSaveError(result == null);
    } catch {
      setSaveError(true);
    }
  };

  return (
    <section className="space-y-4 border-t border-border py-4">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-md bg-muted p-2 text-muted-foreground">
          <Gauge className="h-4 w-4" aria-hidden="true" />
        </div>
        <div>
          <h3 className="text-sm font-medium">
            {t("settings.usageThresholds", {
              defaultValue: "Usage color thresholds",
            })}
          </h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.usageThresholdsDescription", {
              defaultValue:
                "Choose when remaining subscription quota changes from green to yellow or red.",
            })}
          </p>
        </div>
      </div>

      <div className="space-y-4 rounded-lg border border-border/60 bg-muted/20 p-4">
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="usage-warning-threshold">
              {t("settings.usageWarningThreshold", {
                defaultValue: "Yellow at or below",
              })}
            </Label>
            <output
              htmlFor="usage-warning-threshold"
              className="min-w-12 rounded-md bg-warning/15 px-2 py-1 text-center text-xs font-semibold text-warning metric"
              aria-live="polite"
            >
              {warning}%
            </output>
          </div>
          <input
            id="usage-warning-threshold"
            type="range"
            min={0}
            max={100}
            step={1}
            value={warning}
            onChange={(event) => {
              const next = Number(event.target.value);
              setWarning(next);
              setCritical((current) => Math.min(current, next));
            }}
            disabled={isLoading || isSaving || !settings}
            className="h-2 w-full cursor-pointer accent-warning disabled:cursor-not-allowed disabled:opacity-50"
          />
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="usage-critical-threshold">
              {t("settings.usageCriticalThreshold", {
                defaultValue: "Red below",
              })}
            </Label>
            <output
              htmlFor="usage-critical-threshold"
              className="min-w-12 rounded-md bg-danger/15 px-2 py-1 text-center text-xs font-semibold text-danger metric"
              aria-live="polite"
            >
              {critical}%
            </output>
          </div>
          <input
            id="usage-critical-threshold"
            type="range"
            min={0}
            max={warning}
            step={1}
            value={critical}
            onChange={(event) => setCritical(Number(event.target.value))}
            disabled={isLoading || isSaving || !settings}
            className="h-2 w-full cursor-pointer accent-danger disabled:cursor-not-allowed disabled:opacity-50"
          />
        </div>
      </div>

      <p className="text-xs text-muted-foreground">
        {t("settings.usageThresholdsPreview", {
          green: warning,
          yellowLow: critical,
          yellowHigh: warning,
          red: critical,
          defaultValue:
            "Green above {{green}}% · yellow from {{yellowLow}}% to {{yellowHigh}}% · red below {{red}}%",
        })}
      </p>

      {saveError ? (
        <p role="alert" className="text-xs text-danger">
          {t("settings.usageThresholdsSaveFailed", {
            defaultValue: "Could not save usage color thresholds.",
          })}
        </p>
      ) : null}

      <Button
        size="sm"
        onClick={() => void save()}
        disabled={!settings || isLoading || isSaving}
      >
        {t("settings.saveUsageThresholds", {
          defaultValue: "Save thresholds",
        })}
      </Button>
    </section>
  );
}
