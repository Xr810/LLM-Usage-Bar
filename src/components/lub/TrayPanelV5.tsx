import { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { ensureProCopy } from "@/i18n/proCopy";
import { CommunityAllowPlaceholder } from "./CommunityAllowPlaceholder";
import { CommunityBuildBadge } from "./CommunityBuildBadge";
import { communityChromeVisible } from "./detectLubBuild";
import { formatLocalMidnight } from "./money";
import { proCompareEnabled as proCompareDefault } from "./proCompare";
import { SessionList } from "./SessionList";
import { SourceList } from "./SourceList";
import { StatusBanner } from "./StatusBanner";
import { TotalCard } from "./TotalCard";
import type { LubBuild, LubPanelModel, LubTier } from "./panelModel";

interface TrayPanelV5Props {
  model: LubPanelModel;
  locale: string;
  onOpenTrends: () => void;
  onOpenSettings: () => void;
  onRefresh?: () => void;
  onQuit?: () => void;
  refreshing?: boolean;
  showUtilityActions?: boolean;
  openDayTip?: boolean;
  scrollMaxPx?: number;
  refreshError?: string | null;
  /** C2 build. Defaults to official until get_license_state is wired. */
  build?: LubBuild;
  /**
   * C2 tier and upsell_entry. Accepted so callers can pass them through.
   * Neither one shows community chrome or a compare link.
   */
  tier?: LubTier;
  upsellEntry?: boolean;
  /** Batch 1 default is false. The compare line stays out of the banner. */
  proCompareEnabled?: boolean;
}

function PowerIcon() {
  return (
    <svg className="lub-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3v8" />
      <path d="M6.3 6.8a8 8 0 1 0 11.4 0" />
    </svg>
  );
}

function KilledBanner({ model }: { model: LubPanelModel }) {
  const { t } = useTranslation();
  const title = model.betaOn
    ? t("lub.killed.all_spend_stopped_incl_plans", { ns: "pro" })
    : t("lub.killed.all_api_spend_stopped", { ns: "pro" });
  return (
    <div className="lub-banner" role="status">
      <PowerIcon />
      <div className="lub-banner-copy">
        <b>{title}</b>
        <div className="lub-muted">
          {t("lub.killed.stopped_by_you_at_requests", {
            ns: "pro",
            time: model.stoppedAt ?? "",
          })}
        </div>
      </div>
    </div>
  );
}

function LegendCaption({
  model,
  locale,
}: {
  model: LubPanelModel;
  locale: string;
}) {
  const { t } = useTranslation();
  if (model.state === "over") {
    return (
      <p className="lub-legend">
        {t("lub.overcap.caption_resets", {
          time: formatLocalMidnight(locale),
        })}
      </p>
    );
  }
  if (model.betaOn && model.betaPercent != null) {
    return (
      <p className="lub-legend">
        {t("lub.panel.plan_cut_off_beta_is", {
          percent: `${model.betaPercent}%`,
        })}
      </p>
    );
  }
  const text = t("lub.panel.line_spending_stops_there_lighter");
  const [before, after] = text.split("|");
  return (
    <p className="lub-legend">
      {before}
      <span className="lub-vl" aria-hidden="true" />
      {after}
    </p>
  );
}

function SparkGroup({
  model,
  onOpenTrends,
}: {
  model: LubPanelModel;
  onOpenTrends: () => void;
}) {
  const { t } = useTranslation();
  const values = model.spark ?? [];
  const max = Math.max(...values, 0);
  return (
    <button
      type="button"
      className="lub-card lub-card-pad"
      onClick={onOpenTrends}
    >
      <span className="lub-row-between">
        <span className="lub-muted">{t("lub.panel.last_days")}</span>
        <span className="lub-link">{t("lub.panel.trends")} ›</span>
      </span>
      {values.length > 0 ? (
        <span className="lub-spark" aria-hidden="true">
          {values.map((value, index) => (
            <span
              key={index}
              className="lub-spark-bar"
              style={{ height: `${max > 0 ? (value / max) * 100 : 0}%` }}
            />
          ))}
        </span>
      ) : null}
    </button>
  );
}

export function TrayPanelV5({
  model,
  locale,
  onOpenTrends,
  onOpenSettings,
  onRefresh,
  onQuit,
  refreshing = false,
  showUtilityActions = false,
  openDayTip = false,
  scrollMaxPx,
  refreshError,
  build = "official",
  tier = "free",
  upsellEntry = false,
  proCompareEnabled = proCompareDefault,
}: TrayPanelV5Props) {
  ensureProCopy();
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [hiddenRows, setHiddenRows] = useState(0);

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const measure = () => {
      const overflow = node.scrollHeight > node.clientHeight + 1;
      if (!overflow) {
        setHiddenRows(0);
        return;
      }
      const edge = node.getBoundingClientRect().bottom;
      let hidden = 0;
      node.querySelectorAll("[data-lub-row]").forEach((row) => {
        if (row.getBoundingClientRect().top >= edge - 1) hidden += 1;
      });
      setHiddenRows(Math.max(hidden, 1));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [model, scrollMaxPx]);

  const stopLabel = model.betaOn
    ? t("lub.panel.stop_all_spend_incl_plans")
    : t("lub.panel.stop_all_api_spend");
  const community = communityChromeVisible({ build, upsellEntry, tier });

  if (model.state === "killed") {
    const resumeLabel = model.betaOn
      ? t("lub.killed.resume_all_traffic", { ns: "pro" })
      : t("lub.killed.resume_api_traffic", { ns: "pro" });
    return (
      <section
        className="lub-panel"
        data-testid="lub-tray-panel"
        data-state={model.state}
      >
        <KilledBanner model={model} />
        <button
          type="button"
          className="lub-resume"
          aria-disabled="true"
          onClick={(event) => event.preventDefault()}
        >
          {resumeLabel}
        </button>
        <TotalCard model={model} locale={locale} openDayTip={openDayTip} />
        <SourceList model={model} locale={locale} />
        <p className="lub-muted lub-footnote">
          {t("lub.killed.stays_stopped_after_restart_until", { ns: "pro" })}
        </p>
      </section>
    );
  }

  return (
    <section
      className="lub-panel"
      data-testid="lub-tray-panel"
      data-state={model.state}
    >
      {refreshError ? (
        <p className="lub-alert" role="alert">
          {refreshError}
        </p>
      ) : null}
      {model.state === "over" ? (
        <>
          <StatusBanner
            capUsd={model.dailyCapUsd}
            locale={locale}
            proCompareEnabled={proCompareEnabled}
          />
          {community ? <CommunityAllowPlaceholder locale={locale} /> : null}
        </>
      ) : null}
      <TotalCard model={model} locale={locale} openDayTip={openDayTip} />
      <SparkGroup model={model} onOpenTrends={onOpenTrends} />
      <div
        ref={scrollRef}
        className="lub-scroll"
        tabIndex={0}
        role="region"
        aria-label={t("lub.panel.aria_scroll_region")}
        style={scrollMaxPx ? { maxHeight: scrollMaxPx } : undefined}
        onKeyDown={(event) => {
          const row = event.currentTarget.querySelector("[data-lub-row]");
          const step = row?.getBoundingClientRect().height || 40;
          if (event.key === "ArrowDown") {
            event.currentTarget.scrollBy({ top: step });
            event.preventDefault();
          } else if (event.key === "ArrowUp") {
            event.currentTarget.scrollBy({ top: -step });
            event.preventDefault();
          }
        }}
      >
        <div className="lub-scroll-region">
          <SourceList model={model} locale={locale} />
          <SessionList model={model} locale={locale} />
        </div>
        {hiddenRows > 0 ? (
          <div className="lub-scroll-fade" aria-hidden="true" />
        ) : null}
      </div>
      {hiddenRows > 0 ? (
        <p className="lub-more">
          {t("lub.panel.more_rows", { count: hiddenRows })}
        </p>
      ) : null}
      {model.sources.length === 0 ? (
        <p className="lub-empty">
          {t("trayUsage.empty", {
            defaultValue: "No visible Provider accounts",
          })}
        </p>
      ) : null}
      {model.state === "over" ? null : (
        <button
          type="button"
          className="lub-kill"
          aria-disabled="true"
          onClick={(event) => event.preventDefault()}
        >
          <PowerIcon />
          {stopLabel}
        </button>
      )}
      <LegendCaption model={model} locale={locale} />
      <footer className="lub-foot">
        <span className="lub-foot-links">
          <button type="button" className="lub-link" onClick={onOpenSettings}>
            {t("lub.panel.settings")}
          </button>
          <button
            type="button"
            className="lub-link"
            aria-disabled="true"
            onClick={(event) => event.preventDefault()}
          >
            {t("lub.panel.client_receipts")}
          </button>
        </span>
        {model.sample || community || showUtilityActions ? (
          <span className="lub-foot-meta">
            {model.sample ? (
              <span className="lub-muted">{t("lub.panel.sample_data")}</span>
            ) : null}
            {community ? <CommunityBuildBadge /> : null}
            {showUtilityActions ? (
              <span className="lub-utility">
                <button
                  type="button"
                  className="lub-icon-button"
                  onClick={onRefresh}
                  disabled={refreshing}
                  aria-label={
                    refreshing
                      ? t("trayUsage.refreshing", {
                          defaultValue: "Refreshing",
                        })
                      : t("trayUsage.refresh", { defaultValue: "Refresh" })
                  }
                >
                  <RefreshCw
                    className={refreshing ? "animate-spin" : undefined}
                    aria-hidden="true"
                    size={16}
                  />
                </button>
                <button
                  type="button"
                  className="lub-icon-button"
                  onClick={onQuit}
                  aria-label={t("trayUsage.quit", { defaultValue: "Quit" })}
                >
                  <PowerIcon />
                </button>
              </span>
            ) : null}
          </span>
        ) : null}
      </footer>
    </section>
  );
}
