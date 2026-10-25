import { useState } from "react";
import { useTranslation } from "react-i18next";

/** C5 mock. Every tier shows this many weeks. Not a Pro lock. */
export const HEATMAP_MAX_WEEKS = 52;

/** Second-batch Pro feature. Batch 1 mocks omit it, so the card stays hidden. */
export const WEEKDAY_HOUR_FEATURE = "heatmap_weekday_hour";

const WEEKDAYS = [0, 1, 2, 3, 4, 5, 6] as const;
const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;
const WEEKDAY_LABELS_EN = [
  "Mon",
  "Tue",
  "Wed",
  "Thu",
  "Fri",
  "Sat",
  "Sun",
] as const;

const CHART = [
  42, 58, 36, 70, 64, 48, 22, 55, 61, 74, 80, 44, 38, 66, 52, 30, 48, 40, 28,
  58, 36, 46, 62, 34, 50, 44, 56, 41,
];

const NAV = [
  { id: "trends", zh: "趋势与热力图", en: "Trends & heatmap" },
  { id: "caps", zh: "限额与停止", en: "Caps & stop" },
  { id: "providers", zh: "Provider 接入", en: "Providers" },
  { id: "alerts", zh: "提醒", en: "Alerts" },
  { id: "license", zh: "License", en: "License" },
  { id: "failure", zh: "故障策略", en: "Failure policy" },
  { id: "clients", zh: "客户与导出", en: "Clients & export" },
  { id: "about", zh: "关于", en: "About" },
] as const;

function heatLevel(week: number, weekday: number): 0 | 1 | 2 | 3 | 4 {
  const n = (week * 3 + weekday * 5) % 9;
  return (n > 4 ? n - 5 : n) as 0 | 1 | 2 | 3 | 4;
}

export function TrendsPage({
  features = [],
}: {
  features?: readonly string[];
}) {
  const { t, i18n } = useTranslation();
  const zh = i18n.language.startsWith("zh");
  const showWeekday = features.includes(WEEKDAY_HOUR_FEATURE);
  const [range, setRange] = useState<"day" | "week" | "month">("day");
  const weekdayNames = zh ? WEEKDAY_LABELS : WEEKDAY_LABELS_EN;

  return (
    <div className="lub-trends" data-testid="lub-trends">
      <nav className="lub-trends-nav" aria-label={t("lub.trends.day")}>
        {NAV.map((item) => (
          <button
            key={item.id}
            type="button"
            className="lub-trends-nav-item"
            aria-current={item.id === "trends" ? "page" : undefined}
            onClick={(event) => event.preventDefault()}
          >
            {zh ? item.zh : item.en}
          </button>
        ))}
      </nav>
      <div className="lub-trends-main">
        <h1 className="lub-title">
          {zh ? "趋势与热力图" : "Trends & heatmap"}
        </h1>
        <div className="lub-trends-toolbar">
          <div className="lub-trends-seg" role="group">
            {(["day", "week", "month"] as const).map((key) => (
              <button
                key={key}
                type="button"
                className="lub-trends-seg-btn"
                aria-pressed={range === key}
                onClick={() => setRange(key)}
              >
                {t(`lub.trends.${key}`)}
              </button>
            ))}
          </div>
          <span className="lub-trends-seg-btn" aria-current="true">
            Provider
          </span>
        </div>
        <div className="lub-trends-stats">
          <div>
            <span className="lub-muted">
              {zh ? "近 30 天" : "Last 30 days"}
            </span>
            <b>$159.04</b>
          </div>
          <div>
            <span className="lub-muted">{t("lub.trends.daily_avg")}</span>
            <b>$5.30</b>
          </div>
          <div>
            <span className="lub-muted">{t("lub.trends.peak_day")}</span>
            <b>$8.72</b>
          </div>
        </div>
        <section className="lub-card lub-trends-card">
          <div className="lub-row-between">
            <span className="lub-muted">
              {t("lub.trends.api_spend_usd_plans_excluded")}
            </span>
            <span className="lub-trends-legend">
              <i data-series="a" /> Anthropic
              <i data-series="o" /> OpenAI
              <i data-series="r" /> OpenRouter
            </span>
          </div>
          <div className="lub-trends-chart" aria-hidden="true">
            {CHART.map((value, index) => (
              <span key={index} style={{ height: `${value}%` }}>
                <i data-series="r" style={{ flex: 1 }} />
                <i data-series="o" style={{ flex: 1.4 }} />
                <i data-series="a" style={{ flex: 2 }} />
              </span>
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
              {weekdayNames.map((label, index) => (
                <span key={label}>{index % 2 === 0 ? label : ""}</span>
              ))}
            </div>
            <div
              className="lub-heat-grid"
              style={{
                gridTemplateColumns: `repeat(${HEATMAP_MAX_WEEKS}, 1fr)`,
              }}
            >
              {Array.from({ length: HEATMAP_MAX_WEEKS }, (_, week) =>
                WEEKDAYS.map((weekday) => (
                  <i
                    key={`${week}-${weekday}`}
                    data-level={heatLevel(week, weekday)}
                    style={{ gridColumn: week + 1, gridRow: weekday + 1 }}
                  />
                )),
              )}
            </div>
          </div>
        </section>
        {showWeekday ? (
          <section
            className="lub-card lub-trends-card"
            aria-label={t("lub.trends.weekday_by_hour")}
          >
            <div className="lub-row-between">
              <span>{t("lub.trends.weekday_by_hour")}</span>
              <span className="lub-muted">{t("lub.trends.peak_tue_h")}</span>
            </div>
          </section>
        ) : null}
      </div>
    </div>
  );
}
