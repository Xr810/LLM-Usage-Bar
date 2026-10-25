import { useTranslation } from "react-i18next";
import { ensureProCopy } from "@/i18n/proCopy";
import { CapBar } from "./CapBar";
import { CapMarker } from "./CapMarker";
import { parseNonNegative } from "./capMath";
import { splitUsd } from "./money";
import type { LubPanelModel, LubSourceModel } from "./panelModel";

function SourceRow({
  source,
  locale,
  betaOn,
  killed,
}: {
  source: LubSourceModel;
  locale: string;
  betaOn: boolean;
  killed: boolean;
}) {
  ensureProCopy();
  const { t } = useTranslation();
  const money = splitUsd(source.amountUsd, locale, source.costQuality);
  const percent =
    source.usedPercent == null ? null : `${Math.round(source.usedPercent)}%`;
  const valueText =
    percent ?? (money.approximate ? `≈${money.text}` : money.text);
  const cap = parseNonNegative(source.scaleCapUsd);
  const value =
    source.usedPercent != null
      ? source.usedPercent
      : (parseNonNegative(source.amountUsd) ?? 0);
  const showBeta =
    betaOn && source.betaCapPercent != null && source.usedPercent != null;
  // Beta off: only API rows stop. Beta on: proxied plans stop too.
  const rowStopped = killed && (source.enforceable || betaOn);
  const stoppedLabel = t("lub.killed.stopped", { ns: "pro" });
  const rowLabel =
    showBeta || source.enforceable
      ? t("lub.panel.aria_row_cap_enforced", { name: source.name })
      : t("lub.panel.aria_row_monitor_only", { name: source.name });
  const quotaGapLabel =
    source.quotaGap === "error"
      ? t("lub.panel.quota_read_failed")
      : source.quotaGap === "stale"
        ? t("lub.panel.quota_stale")
        : null;
  const valueNode = quotaGapLabel ? (
    <span className="lub-muted">{quotaGapLabel}</span>
  ) : rowStopped ? (
    <span className="lub-chip">{stoppedLabel}</span>
  ) : killed && percent ? (
    <span className="lub-muted">
      {t("lub.killed.running", { ns: "pro", percent })}
    </span>
  ) : (
    <span className="lub-value">{valueText}</span>
  );

  return (
    <div className="lub-source-row" data-lub-row="">
      <div className="lub-row-between">
        <span className="lub-name">
          <span className="lub-mono" aria-hidden="true">
            {source.monogram}
          </span>
          <span className="lub-name-text">
            {source.name}
            <small>{t(`lub.panel.${source.subtitleKey}`)}</small>
          </span>
        </span>
        {valueNode}
      </div>
      {quotaGapLabel ? null : source.onSetBudget ? (
        <button type="button" className="lub-link" onClick={source.onSetBudget}>
          {t("trayUsage.setProviderBudget", {
            provider: source.name,
            defaultValue: "Set daily budget for {{provider}}",
          })}
        </button>
      ) : (
        <CapBar
          value={value}
          cap={source.usedPercent != null ? 100 : cap}
          scale={source.usedPercent != null ? "raw" : "cap"}
          variant="row"
          tone={rowStopped ? "muted" : "accent"}
          label={rowLabel}
          valueText={rowStopped ? stoppedLabel : valueText}
          marker={
            showBeta ? (
              <CapMarker
                kind="beta"
                leftRatio={(source.betaCapPercent ?? 90) / 100}
                label={t("lub.panel.aria_cap_line_beta", {
                  percent: `${source.betaCapPercent}%`,
                })}
                tooltip={
                  killed
                    ? t("lub.killed.stopped_by_you_beta", { ns: "pro" })
                    : t("lub.panel.stops_at_beta", {
                        percent: `${source.betaCapPercent}%`,
                      })
                }
              />
            ) : null
          }
        />
      )}
    </div>
  );
}

export function SourceList({
  model,
  locale,
}: {
  model: LubPanelModel;
  locale: string;
}) {
  const { t } = useTranslation();
  if (model.sources.length === 0) return null;
  const killed = model.state === "killed";
  return (
    <section aria-label={t("lub.panel.traffic_sources")}>
      {killed ? null : (
        <h2 className="lub-section-title">{t("lub.panel.traffic_sources")}</h2>
      )}
      <div className="lub-card">
        {model.sources.map((source) => (
          <SourceRow
            key={source.id}
            source={source}
            locale={locale}
            betaOn={model.betaOn}
            killed={killed}
          />
        ))}
      </div>
    </section>
  );
}
