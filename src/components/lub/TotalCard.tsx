import { useTranslation } from "react-i18next";
import { ensureProCopy } from "@/i18n/proCopy";
import { CapBar } from "./CapBar";
import { CapMarker } from "./CapMarker";
import { parseNonNegative } from "./capMath";
import { displayUsd, splitUsd } from "./money";
import type { LubPanelModel } from "./panelModel";

interface TotalCardProps {
  model: LubPanelModel;
  locale: string;
  openDayTip?: boolean;
}

export function TotalCard({
  model,
  locale,
  openDayTip = false,
}: TotalCardProps) {
  ensureProCopy();
  const { t } = useTranslation();
  const stopped = model.state === "killed";
  const stoppedLabel = t("lub.killed.stopped", { ns: "pro" });
  const stoppedTip = t("lub.killed.stopped_by_you_tap_resume", { ns: "pro" });
  const today = splitUsd(model.todayUsd, locale, model.todayQuality);
  const dailyCap = splitUsd(model.dailyCapUsd, locale, "complete");
  const dailyCapText = displayUsd(dailyCap.text);
  const month = splitUsd(model.monthUsd, locale, model.monthQuality);
  const monthCap = splitUsd(model.monthlyCapUsd, locale, "complete");
  const monthCapText = displayUsd(monthCap.text);
  const todayValue = parseNonNegative(model.todayUsd) ?? 0;
  const dailyCapValue = parseNonNegative(model.dailyCapUsd);
  const monthValue = parseNonNegative(model.monthUsd) ?? 0;
  const monthCapValue = parseNonNegative(model.monthlyCapUsd);
  const over = model.state === "over";
  const todayText = displayUsd(today.text);
  const statusLabel = stopped
    ? stoppedLabel
    : over
      ? t("lub.overcap.status_over_cap")
      : model.state === "capped" && dailyCapText !== "—"
        ? t("lub.panel.stops_at_amt", { amount: dailyCapText })
        : t("lub.panel.protected");
  const dayLabel = stopped
    ? `${stoppedLabel}. ${stoppedTip}`
    : over
      ? t("lub.overcap.aria_total_over", {
          amount: todayText,
          amount2: dailyCapText,
        })
      : t("lub.panel.aria_cap_line_day", { amount: dailyCapText });
  const dayTip = stopped
    ? stoppedTip
    : over
      ? t("lub.overcap.tip_cap_alert_only", { amount: dailyCapText })
      : t("lub.panel.stops_at_amt", { amount: dailyCapText });

  return (
    <div className="lub-hero">
      <div className="lub-row-between">
        <span className="lub-label">{t("lub.panel.today_s_api_spend")}</span>
        <span
          className={
            stopped
              ? "lub-status lub-status-stop"
              : over
                ? "lub-status lub-status-warn"
                : "lub-status"
          }
        >
          <i className={`lub-dot lub-dot-${model.state}`} aria-hidden="true" />
          <span>{statusLabel}</span>
        </span>
      </div>
      <div className="lub-hero-amount">
        {today.approximate ? <span className="lub-approx">≈</span> : null}
        {today.symbol ? (
          <span className="lub-hero-symbol">{today.symbol}</span>
        ) : null}
        <span className="lub-hero-number">{today.number}</span>
        {model.dailyCapUsd ? (
          <span className="lub-hero-of">/ {dailyCapText}</span>
        ) : null}
      </div>
      <CapBar
        value={todayValue}
        cap={dailyCapValue}
        scale="cap"
        variant="day"
        tone={stopped ? "muted" : "accent"}
        overflow={over ? "accent" : "stop"}
        label={dayLabel}
        valueText={t("lub.panel.today_s_api_spend")}
        marker={
          model.showDayLine ? (
            <CapMarker
              kind={stopped ? "hit" : "day"}
              defaultOpen={openDayTip}
              label={dayLabel}
              tooltip={dayTip}
            />
          ) : null
        }
      />
      {model.onSetSharedBudget ? (
        <button
          type="button"
          className="lub-link"
          onClick={model.onSetSharedBudget}
        >
          {t("trayUsage.setCombinedApiBudget", {
            defaultValue: "Set combined API daily limit",
          })}
        </button>
      ) : null}
      <div className="lub-row-between lub-month-head">
        <span className="lub-muted">{t("lub.panel.this_month")}</span>
        <span className="lub-value lub-muted">
          {month.approximate ? "≈" : null}
          {month.text}
          {model.monthlyCapUsd ? <span> / {monthCapText}</span> : null}
        </span>
      </div>
      <CapBar
        value={monthValue}
        cap={monthCapValue}
        scale="cap"
        variant="thin"
        tone="accent"
        label={t("lub.panel.aria_cap_line_month", { amount: monthCapText })}
        valueText={t("lub.panel.this_month")}
        marker={
          model.showMonthLine ? (
            <CapMarker
              kind="month"
              label={t("lub.panel.aria_cap_line_month", {
                amount: monthCapText,
              })}
              tooltip={
                over
                  ? t("lub.overcap.tip_month_alert_only", {
                      amount: monthCapText,
                    })
                  : t("lub.panel.stops_at_amt_this_month", {
                      amount: monthCapText,
                    })
              }
            />
          ) : null
        }
      />
    </div>
  );
}
