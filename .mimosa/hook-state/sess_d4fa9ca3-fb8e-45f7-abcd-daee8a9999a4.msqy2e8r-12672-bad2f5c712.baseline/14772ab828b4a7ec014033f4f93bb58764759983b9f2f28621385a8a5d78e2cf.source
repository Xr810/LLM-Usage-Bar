import { useState } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { QuotaWindowPaceView } from "@/types/usageDashboard";

/**
 * The reasoning behind a quota window's colour, on demand.
 *
 * Collapsed by default: the colour is the answer most of the time, and two of
 * these per card would drown the numbers people actually came for. It opens
 * when the colour is surprising enough to want justifying.
 */
export function QuotaPaceDetails({
  pace,
  resetsAt,
}: {
  pace: QuotaWindowPaceView | undefined;
  resetsAt: string | null | undefined;
}) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);

  // Nothing worth opening a disclosure for: no clock to project against, so
  // the verdict is the plain remaining-percentage fallback and says itself.
  if (!pace || pace.paceBasis == null || pace.paceBasis === "static") {
    return null;
  }

  const locale = i18n.resolvedLanguage ?? i18n.language;
  const rows: { key: string; label: string; value: string }[] = [];

  const burnRate = Number(pace.burnRatePercentPerHour);
  if (Number.isFinite(burnRate)) {
    rows.push({
      key: "rate",
      label: t("usageDashboard.paceBurnRate", { defaultValue: "Burn rate" }),
      value:
        pace.paceBasis === "idle" || burnRate === 0
          ? t("usageDashboard.paceIdle", { defaultValue: "Idle" })
          : t("usageDashboard.pacePercentPerHour", {
              value: burnRate.toFixed(burnRate < 10 ? 1 : 0),
              defaultValue: "{{value}}%/h",
            }),
    });
  }

  const exhaustMs = pace.projectedExhaustAt
    ? Date.parse(pace.projectedExhaustAt)
    : Number.NaN;
  const resetMs = resetsAt ? Date.parse(resetsAt) : Number.NaN;
  if (Number.isFinite(exhaustMs)) {
    // Whether it lands before or after the reset is the whole verdict, so lead
    // with that rather than making the reader compare two timestamps.
    const survivesReset = Number.isFinite(resetMs) && exhaustMs >= resetMs;
    rows.push({
      key: "exhaust",
      label: t("usageDashboard.paceRunsOut", { defaultValue: "Runs out" }),
      value: survivesReset
        ? t("usageDashboard.paceAfterReset", {
            defaultValue: "After the reset",
          })
        : new Intl.DateTimeFormat(locale, { timeStyle: "short" }).format(
            exhaustMs,
          ),
    });
  } else if (pace.paceBasis === "idle") {
    rows.push({
      key: "exhaust",
      label: t("usageDashboard.paceRunsOut", { defaultValue: "Runs out" }),
      value: t("usageDashboard.paceNotAtThisRate", {
        defaultValue: "Not at this rate",
      }),
    });
  }

  const rhythm = Number(pace.rhythmAdjustment);
  if (Number.isFinite(rhythm) && rhythm > 0 && rhythm !== 1) {
    rows.push({
      key: "rhythm",
      label: t("usageDashboard.paceRhythm", { defaultValue: "Your rhythm" }),
      value:
        rhythm < 1
          ? t("usageDashboard.paceRhythmQuieter", {
              value: Math.round((1 - rhythm) * 100),
              defaultValue: "Usually {{value}}% quieter now",
            })
          : t("usageDashboard.paceRhythmBusier", {
              value: Math.round((rhythm - 1) * 100),
              defaultValue: "Usually {{value}}% busier now",
            }),
    });
  }

  rows.push({
    key: "basis",
    label: t("usageDashboard.paceBasis", { defaultValue: "Based on" }),
    value:
      pace.paceBasis === "window_average"
        ? t("usageDashboard.paceBasisWindowAverage", {
            defaultValue: "This window's average — not enough recent samples",
          })
        : t("usageDashboard.paceBasisMeasured", {
            defaultValue: "Your measured rate over the last 30 minutes",
          }),
  });

  if (rows.length === 0) return null;

  return (
    <div className="mt-1">
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        className="flex items-center gap-1 text-[11px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <ChevronRight
          className={`h-3 w-3 transition-transform ${open ? "rotate-90" : ""}`}
          aria-hidden="true"
        />
        {t("usageDashboard.paceWhyThisColour", {
          defaultValue: "Why this colour?",
        })}
      </button>
      {open ? (
        <dl className="mt-1.5 space-y-1 border-l border-border/60 pl-2.5 text-[11px]">
          {rows.map((row) => (
            <div key={row.key} className="flex gap-2">
              <dt className="w-20 shrink-0 text-muted-foreground">
                {row.label}
              </dt>
              <dd className="min-w-0">{row.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
    </div>
  );
}
