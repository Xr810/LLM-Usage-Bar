import { Gauge } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SettingsSection } from "./SettingsSection";

/**
 * Explains how the usage light decides its colour. Deliberately has no
 * controls: "will this quota run out before it resets" has an objective
 * answer, so it is not something to hand the user a slider for. The pace
 * error band is a product constant rather than a persisted preference.
 */
export function UsageLightInfo() {
  const { t } = useTranslation();

  const verdicts = [
    {
      key: "green",
      dot: "bg-success",
      label: t("settings.usagePaceGreen", { defaultValue: "Green" }),
      body: t("settings.usagePaceGreenBody", {
        defaultValue:
          "Projected to still have quota left when the window resets.",
      }),
    },
    {
      key: "yellow",
      dot: "bg-warning",
      label: t("settings.usagePaceYellow", { defaultValue: "Yellow" }),
      body: t("settings.usagePaceYellowBody", {
        defaultValue: "Projected to run out right around the reset.",
      }),
    },
    {
      key: "red",
      dot: "bg-danger",
      label: t("settings.usagePaceRed", { defaultValue: "Red" }),
      body: t("settings.usagePaceRedBody", {
        defaultValue: "Projected to run out before the window resets.",
      }),
    },
  ];

  const footnotes = [
    t("settings.usagePaceExhaustedNote", {
      defaultValue:
        "Once the quota is spent the light is red whatever the pace — you cannot send anything — unless the reset is under 15 minutes away.",
    }),
    t("settings.usagePaceNoResetNote", {
      defaultValue:
        "A few Providers do not report when the quota resets. With no clock to project against, those fall back to plain remaining-percentage bands.",
    }),
  ];

  return (
    <SettingsSection
      icon={Gauge}
      title={t("settings.usageThresholds", { defaultValue: "Usage light" })}
      description={t("settings.usageThresholdsDescription", {
        defaultValue:
          "The light projects when your quota runs out at your recent burn rate, then compares that to the reset time. Slow down and it turns green again.",
      })}
    >
      <ul className="space-y-1.5 pl-11">
        {verdicts.map((verdict) => (
          <li key={verdict.key} className="flex items-start gap-2.5 text-xs">
            <span
              className={`mt-1 h-2 w-2 shrink-0 rounded-full ${verdict.dot}`}
              aria-hidden="true"
            />
            <span>
              <span className="font-medium">{verdict.label}</span>
              <span className="text-muted-foreground"> — {verdict.body}</span>
            </span>
          </li>
        ))}
      </ul>

      <div className="space-y-1.5 pl-11">
        {footnotes.map((note) => (
          <p key={note} className="text-xs text-muted-foreground/80">
            {note}
          </p>
        ))}
      </div>
    </SettingsSection>
  );
}
