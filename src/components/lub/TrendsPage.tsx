import { useState } from "react";
import { useTranslation } from "react-i18next";
import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import { ensureProCopy } from "@/i18n/proCopy";
import { HEATMAP_MAX_WEEKS } from "@/lib/constants";
import { loadPerGroupDailySeries } from "./perGroupDailyTrend";
import { UpsellPanel } from "./UpsellPanel";
import { useTrendsData } from "./useTrendsData";
import {
  axisLabels,
  chartScale,
  showsWeekdayHour,
  UPSELL_ENTRY_FEATURE,
  weekdayLabels,
  WEEK_STARTS_ON_MONDAY,
  type TrendDimension,
  type TrendGranularity,
} from "./trendsModel";

const NAV = [
  ["trends", "lub.settings.trends_heatmaps"],
  ["caps", "lub.settings.caps_stopping"],
  ["providers", "lub.settings.providers"],
  ["alerts", "lub.settings.alerts"],
  ["failure", "lub.settings.failure_policy"],
  ["clients", "lub.settings.clients_export"],
  ["about", "lub.settings.about"],
] as const;

const TIME_KEYS: { id: TrendGranularity; key: string }[] = [
  { id: "day", key: "lub.trends.day" },
  { id: "week", key: "lub.trends.week" },
  { id: "month", key: "lub.trends.month" },
];

const LOCKED_TIME = new Set<TrendGranularity>(["week", "month"]);
const LOCKED_DIMENSION = new Set<TrendDimension>(["project", "client"]);

const MISSING = "—";

function usdText(amount: number | null, locale: string): string {
  if (amount == null) return MISSING;
  return formatUsd(amount.toFixed(6), locale);
}

export function TrendsPage({
  tier = "free",
  features = [],
}: {
  tier?: "free" | "pro";
  features?: readonly string[];
}) {
  ensureProCopy();
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const phase2Locks = tier !== "pro" && features.includes(UPSELL_ENTRY_FEATURE);
  const showControls = tier === "pro" || phase2Locks;
  const showWeekday = showsWeekdayHour(features);
  const [granularity, setGranularity] = useState<TrendGranularity>("day");
  const [dimension, setDimension] = useState<TrendDimension>("provider");
  const [upsellOpen, setUpsellOpen] = useState(false);
  const activeGranularity: TrendGranularity =
    tier === "pro" ? granularity : "day";
  const activeDimension: TrendDimension =
    tier === "pro" ? dimension : "provider";
  const data = useTrendsData(activeDimension, activeGranularity);
  const scale = chartScale(data.lines);
  const dates = axisLabels(data.points, locale);
  const weekdays = weekdayLabels(locale, data.weekStartsOn);
  const maxLine = data.lines.find((line) => line.kind === "max");
  const capLine = data.lines.find((line) => line.kind === "cap");

  const openUpsell = () => setUpsellOpen(true);

  const dimensionOptions: { id: TrendDimension; label: string }[] = [
    { id: "provider", label: t("lub.settings.providers") },
    { id: "project", label: t("lub.upsell.dim_project", { ns: "pro" }) },
    { id: "client", label: t("lub.upsell.dim_client", { ns: "pro" }) },
  ].filter(
    (item) =>
      item.id === "provider" || loadPerGroupDailySeries(item.id) != null,
  );

  return (
    <div
      className="lub-trends"
      data-testid="lub-trends"
      data-tier={tier}
      data-week-start={WEEK_STARTS_ON_MONDAY}
      data-max-weeks={HEATMAP_MAX_WEEKS}
      data-truncated={data.truncated ? "true" : "false"}
      data-per-group={data.perGroupDaily == null ? "none" : "day"}
    >
      <nav className="lub-trends-nav" aria-label={t("lub.panel.settings")}>
        {NAV.map(([id, key]) => (
          <button
            key={id}
            type="button"
            className="lub-trends-nav-item"
            aria-current={id === "trends" ? "page" : undefined}
            onClick={(event) => event.preventDefault()}
          >
            {t(key)}
          </button>
        ))}
      </nav>
      <div className="lub-trends-main">
        <h1 className="lub-title">{t("lub.settings.trends_heatmaps")}</h1>
        {showControls ? (
          <div className="lub-trends-toolbar" data-testid="lub-trends-toolbar">
            <div className="lub-trends-seg">
              {TIME_KEYS.map((item) => {
                const locked = phase2Locks && LOCKED_TIME.has(item.id);
                const label = t(item.key);
                return (
                  <button
                    key={item.id}
                    type="button"
                    className="lub-trends-seg-btn"
                    aria-pressed={!locked && activeGranularity === item.id}
                    aria-label={
                      locked
                        ? t("lub.upsell.aria_pro_tag", {
                            ns: "pro",
                            name: label,
                          })
                        : undefined
                    }
                    onClick={() => {
                      if (locked) {
                        openUpsell();
                        return;
                      }
                      if (tier === "pro") setGranularity(item.id);
                    }}
                  >
                    {label}
                    {locked ? (
                      <span className="lub-pro-tag">
                        {t("lub.upsell.pro_tag", { ns: "pro" })}
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
            <div className="lub-trends-seg">
              {dimensionOptions.map((item) => {
                const locked = phase2Locks && LOCKED_DIMENSION.has(item.id);
                return (
                  <button
                    key={item.id}
                    type="button"
                    className="lub-trends-seg-btn"
                    aria-pressed={!locked && activeDimension === item.id}
                    aria-label={
                      locked
                        ? t("lub.upsell.aria_pro_tag", {
                            ns: "pro",
                            name: item.label,
                          })
                        : undefined
                    }
                    onClick={() => {
                      if (locked) {
                        openUpsell();
                        return;
                      }
                      if (tier === "pro") setDimension(item.id);
                    }}
                  >
                    {item.label}
                    {locked ? (
                      <span className="lub-pro-tag">
                        {t("lub.upsell.pro_tag", { ns: "pro" })}
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          </div>
        ) : null}
        <div className="lub-trends-stats">
          <div>
            <span className="lub-muted">{t("lub.trends.last_30_days")}</span>
            <b data-testid="lub-trend-total">
              {usdText(data.totalUsd, locale)}
            </b>
          </div>
          <div>
            <span className="lub-muted">{t("lub.trends.daily_avg")}</span>
            <b data-testid="lub-trend-average">
              {usdText(data.averageUsd, locale)}
            </b>
          </div>
          <div>
            <span className="lub-muted">{t("lub.trends.peak_day")}</span>
            <b data-testid="lub-trend-peak">{usdText(data.peakUsd, locale)}</b>
          </div>
        </div>
        <section className="lub-card lub-trends-card">
          <div className="lub-row-between">
            <span className="lub-muted">
              {t("lub.trends.api_spend_usd_plans_excluded")}
            </span>
          </div>
          <div
            className="lub-trends-plot"
            role="img"
            aria-label={`${t("lub.trends.api_spend_usd_plans_excluded")} ${t("lub.trends.last_30_days")}`}
          >
            <div className="lub-trends-chart">
              {data.points.map((point) => {
                const height =
                  point.costUsd == null ? 0 : (point.costUsd / scale) * 100;
                return (
                  <span
                    key={point.startAt}
                    data-series="total"
                    style={{ height: `${height}%` }}
                  >
                    {height > 0 ? <i data-slot="0" /> : null}
                  </span>
                );
              })}
            </div>
            {data.lines.map((line) => {
              const percent = (line.amountUsd / scale) * 100;
              const crowded =
                line.kind === "cap" &&
                maxLine != null &&
                capLine != null &&
                Math.abs(capLine.amountUsd - maxLine.amountUsd) / scale < 0.08;
              return (
                <div
                  key={line.kind}
                  className="lub-trends-line"
                  data-kind={line.kind}
                  data-testid={`lub-trend-line-${line.kind}`}
                  data-offset={crowded ? "up" : undefined}
                  style={{ bottom: `${percent}%` }}
                  aria-label={
                    line.kind === "cap"
                      ? t("lub.panel.aria_cap_line_day", {
                          amount: usdText(line.amountUsd, locale),
                        })
                      : undefined
                  }
                >
                  <b>{usdText(line.amountUsd, locale)}</b>
                </div>
              );
            })}
          </div>
          <div className="lub-trends-axis" data-testid="lub-trends-axis">
            {dates.map((label) => (
              <time key={label.startAt} dateTime={label.iso}>
                {label.text}
              </time>
            ))}
          </div>
        </section>
        <section
          className="lub-card lub-trends-card"
          aria-label={t("lub.trends.daily_spend_last_weeks")}
        >
          <div className="lub-row-between">
            <span>{t("lub.trends.daily_spend_last_weeks")}</span>
            <span className="lub-trends-scale">
              {t("lub.trends.less")}
              <i data-level="0" />
              <i data-level="1" />
              <i data-level="2" />
              <i data-level="3" />
              <i data-level="4" />
              {t("lub.trends.more")}
            </span>
          </div>
          <div className="lub-heat">
            <div className="lub-heat-labels">
              {weekdays.map((label, index) => (
                <span key={`${label}-${index}`}>
                  {index % 2 === 0 ? label : ""}
                </span>
              ))}
            </div>
            <div
              className="lub-heat-grid"
              data-max-weeks={data.maxWeeks}
              style={{
                gridTemplateColumns: `repeat(${data.maxWeeks}, 1fr)`,
              }}
            >
              {data.heatmap.map((cell) => (
                <i
                  key={cell.key}
                  data-level={cell.level}
                  style={{
                    gridColumn: cell.week + 1,
                    gridRow: cell.weekday + 1,
                  }}
                />
              ))}
            </div>
          </div>
        </section>
        {showWeekday ? (
          <section
            className="lub-card lub-trends-card"
            aria-label={t("lub.trends.weekday_by_hour")}
            data-testid="lub-weekday-hour"
          >
            <div className="lub-row-between">
              <span>{t("lub.trends.weekday_by_hour")}</span>
              <span className="lub-muted">{t("lub.trends.peak_tue_h")}</span>
            </div>
          </section>
        ) : null}
        {phase2Locks && upsellOpen ? (
          <UpsellPanel onClose={() => setUpsellOpen(false)} />
        ) : null}
      </div>
    </div>
  );
}
