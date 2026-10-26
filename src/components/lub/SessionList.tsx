import { useTranslation } from "react-i18next";
import { CapBar } from "./CapBar";
import { parseNonNegative, reachesWarn } from "./capMath";
import { displayUsd, splitUsd } from "./money";
import type { LubPanelModel } from "./panelModel";

export function SessionList({
  model,
  locale,
}: {
  model: LubPanelModel;
  locale: string;
}) {
  const { t } = useTranslation();
  const sessions = model.sessions ?? [];
  const invalidRows = model.invalidCostRows ?? 0;
  if (sessions.length === 0 && invalidRows <= 0) return null;
  return (
    <section aria-label={t("lub.panel.active_sessions")}>
      <h2 className="lub-section-title">{t("lub.panel.active_sessions")}</h2>
      {invalidRows > 0 ? (
        <p className="lub-muted" role="status">
          {t("lub.panel.invalid_cost_rows", { count: invalidRows })}
        </p>
      ) : null}
      {sessions.length === 0 ? null : (
        <div className="lub-card">
          {sessions.map((session) => {
            const spent = parseNonNegative(session.spentUsd) ?? 0;
            const cap = parseNonNegative(session.capUsd);
            const spentText = splitUsd(session.spentUsd, locale).text;
            const capText = displayUsd(splitUsd(session.capUsd, locale).text);
            const near =
              cap != null && reachesWarn(spent, cap, model.alertThresholdPct);
            const percent =
              cap != null && cap > 0
                ? `${Math.round((spent / cap) * 100)}%`
                : "";
            const client =
              session.clientLabel ??
              (session.clientKey ? t(`lub.panel.${session.clientKey}`) : null);
            const alias =
              session.group === "unattributed"
                ? t("lub.panel.group_unattributed")
                : session.alias;
            return (
              <div key={session.id} className="lub-session-row" data-lub-row="">
                <div className="lub-row-between">
                  <span className="lub-name-text">
                    {alias}
                    {client ? <small>{client}</small> : null}
                  </span>
                  <span className="lub-value">
                    {spentText} <span className="lub-muted">/ {capText}</span>
                    {near ? (
                      <span className="lub-near">
                        {t("lub.panel.session_near_limit")}
                      </span>
                    ) : null}
                  </span>
                </div>
                <CapBar
                  value={spent}
                  cap={cap}
                  scale="raw"
                  variant="row"
                  tone="accent"
                  near={near}
                  tooltip={
                    near
                      ? t("lub.panel.tooltip_session_near_limit", { percent })
                      : undefined
                  }
                  label={
                    near
                      ? t("lub.panel.aria_session_near_limit", {
                          name: alias,
                          used: spentText,
                          cap: capText,
                          percent,
                        })
                      : alias
                  }
                  valueText={`${spentText} / ${capText}`}
                />
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
