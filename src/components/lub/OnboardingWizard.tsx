import { useState } from "react";
import { useTranslation } from "react-i18next";
import { isWindows } from "@/lib/platform";
import {
  CLAUDE_PROXY_TERMS_VERSION,
  detectTools,
  noteCopyKey,
  setClaudeSubscriptionProxy,
  type DetectedTool,
  type ProxyPlatform,
} from "./takeoverMock";

function proxyPlatform(): ProxyPlatform {
  return isWindows() ? "windows" : "mac";
}

function LineCopy({ value }: { value: string }) {
  const [before, rest = ""] = value.split("|");
  const [mid, after = ""] = rest.split('<i class="vl m"></i>');
  return (
    <>
      {before}
      <span className="lub-vl" aria-hidden="true" />
      {mid}
      <span className="lub-vl lub-vl-month" aria-hidden="true" />
      {after}
    </>
  );
}

function ToolRow({ tool }: { tool: DetectedTool }) {
  const { t, i18n } = useTranslation();
  const [checked, setChecked] = useState(true);
  const name = i18n.language.startsWith("zh") ? tool.name : tool.nameEn;
  const tag = tool.enforceable
    ? t("lub.onboard.cap_enforced")
    : t("lub.panel.monitor_only");
  if (tool.control === "none") {
    return (
      <div className="lub-tool-row" data-tool={tool.tool}>
        <span>{name}</span>
        <span className="lub-tag">{tag}</span>
      </div>
    );
  }
  return (
    <label className="lub-tool-row" data-tool={tool.tool}>
      <span>
        <input
          type="checkbox"
          checked={checked}
          onChange={(event) => setChecked(event.target.checked)}
        />{" "}
        {name}
        {tool.detailKey ? (
          <span className="lub-muted"> {t(`lub.panel.${tool.detailKey}`)}</span>
        ) : null}
      </span>
      <span className="lub-tag">{tag}</span>
    </label>
  );
}

export function OnboardingWizard() {
  const { t } = useTranslation();
  const tools = detectTools();
  const notes = tools.flatMap((tool) => tool.noteCodes);
  const [acked, setAcked] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const terms = t("lub.onboard.terms_risk");
  const proxyLabel = t("lub.onboard.claude_subscription_via_proxy");

  return (
    <div className="lub-modal" data-testid="onboarding-wizard">
      <div className="lub-steps" aria-hidden="true">
        <i className="on" />
        <i className="on" />
        <i />
      </div>
      <p className="lub-muted">{t("lub.onboard.step_of")}</p>
      <h1 className="lub-title">
        {t("lub.onboard.connect_your_coding_tools")}
      </h1>
      <p className="lub-muted">
        {t("lub.onboard.we_found_these_tools_checked")}
      </p>
      <div className="lub-card lub-pad">
        {tools.map((tool) => (
          <ToolRow key={tool.tool} tool={tool} />
        ))}
      </div>
      <p className="lub-muted">
        <LineCopy value={t("lub.onboard.the_total_bar_in_the")} />
      </p>
      {notes.length > 0 ? (
        <div className="lub-banner-warn" role="status">
          <svg className="lub-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 4 2.8 19.5h18.4z" />
            <path d="M12 10v4.5" />
            <path d="M12 17.3v.2" />
          </svg>
          <div>
            <b>{t("lub.onboard.check_up_found_risk")}</b>
            {notes.map((code) => {
              const key = noteCopyKey(code);
              return (
                <div key={code} className="lub-muted">
                  {key ? t(key) : code}
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
      <label className="lub-muted">
        <input
          type="checkbox"
          checked={acked}
          onChange={(event) => {
            const next = event.target.checked;
            setAcked(next);
            if (next || !enabled) return;
            const result = setClaudeSubscriptionProxy({
              enabled: false,
              platform: proxyPlatform(),
            });
            if (!result.ok) {
              setError(result.error.code);
              return;
            }
            setError(null);
            setEnabled(false);
          }}
        />{" "}
        {terms}
      </label>
      <div className="lub-tool-row">
        <span>{proxyLabel}</span>
        <button
          type="button"
          className="lub-switch"
          role="switch"
          aria-checked={enabled}
          aria-label={proxyLabel}
          onClick={() => {
            const next = !enabled;
            const result = next
              ? setClaudeSubscriptionProxy({
                  enabled: true,
                  termsAckAt: acked ? Date.now() : null,
                  termsVersion: acked ? CLAUDE_PROXY_TERMS_VERSION : null,
                  platform: proxyPlatform(),
                })
              : setClaudeSubscriptionProxy({
                  enabled: false,
                  platform: proxyPlatform(),
                });
            if (!result.ok) {
              setEnabled(false);
              setError(result.error.code);
              return;
            }
            setError(null);
            setEnabled(next);
          }}
        />
      </div>
      {error ? (
        <p className="lub-alert" role="alert">
          {error}
        </p>
      ) : null}
      <div className="lub-actions">
        <button
          type="button"
          className="lub-btn"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.onboard.back")}
        </button>
        <button
          type="button"
          className="lub-btn lub-btn-primary"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.onboard.connect_set_daily_cap")}
        </button>
      </div>
    </div>
  );
}
