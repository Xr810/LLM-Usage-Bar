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
  if (model.sessions == null || model.sessions.length === 0) return null;
  return (
    <section aria-label={t("lub.panel.active_sessions")}>
      <h2 className="lub-section-title">{t("lub.panel.active_sessions")}</h2>
      <div className="lub-card">
        {model.sessions.map((session) => {
          const spent = parseNonNegative(session.spentUsd) ?? 0;
          const cap = parseNonNegative(session.capUsd);
          const spentText = splitUsd(session.spentUsd, locale).text;
          const capText = displayUsd(splitUsd(session.capUsd, locale).text);
          const near =
            cap != null && reachesWarn(spent, cap, model.alertThresholdPct);
          const percent =
            cap != null && cap > 0 ? `${Math.round((spent / cap) * 100)}%` : "";
          const client =
            session.clientLabel ??
            (session.clientKey ? t(`lub.panel.${session.clientKey}`) : null);
          return (
            <div key={session.id} className="lub-session-row" data-lub-row="">
              <div className="lub-row-between">
                <span className="lub-name-text">
                  {session.alias}
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
                        name: session.alias,
                        used: spentText,
                        cap: capText,
                        percent,
                      })
                    : session.alias
                }
                valueText={`${spentText} / ${capText}`}
              />
            </div>
          );
        })}
      </div>
    </section>
  );
}
