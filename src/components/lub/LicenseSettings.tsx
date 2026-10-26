import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  activateLicense,
  communityLicenseStatus,
  type LicenseStatus,
} from "./licenseMock";
import { proCompareEnabled } from "./proCompare";

const SAMPLE_TOKEN = "ULB1.eyJ0aWVyIjoicHJvIi···";

const NAV = [
  ["lub.settings.trends_heatmaps", "趋势与热力图", "Trends & heatmaps"],
  ["lub.settings.caps_stopping", "限额与停止", "Caps & stopping"],
  ["lub.settings.providers", "Provider 接入", "Providers"],
  ["lub.settings.alerts", "提醒", "Alerts"],
  ["license", "License", "License"],
  ["lub.settings.failure_policy", "故障策略", "Failure policy"],
  ["lub.settings.clients_export", "客户与导出", "Clients & export"],
  ["lub.settings.about", "关于", "About"],
] as const;

function localizedDefault(language: string, zh: string, en: string): string {
  return language.startsWith("zh") ? zh : en;
}

function StatusValue({ status }: { status: LicenseStatus }) {
  const { t } = useTranslation();
  const label =
    status.build === "community"
      ? t("lub.license.community_build", { defaultValue: "Community build" })
      : status.tier;
  return <span data-testid="license-tier">{label}</span>;
}

export function LicenseSettings({
  variant = "window",
}: {
  variant?: "window" | "embedded";
}) {
  const { t, i18n } = useTranslation();
  const [status] = useState(communityLicenseStatus);
  const [token, setToken] = useState(SAMPLE_TOKEN);
  const [notice, setNotice] = useState<string | null>(null);
  const prompt = t("lub.license.official_build_required", {
    defaultValue: localizedDefault(
      i18n.language,
      "需官方构建",
      "Official build required",
    ),
  });

  const form = (
    <>
      <h1 className="lub-title">License</h1>
      <div className="lub-card lub-pad">
        <p className="lub-muted">
          {t("lub.license.paste_the_license_from_your")}
        </p>
        <div className="lub-field">
          <input
            className="lub-input"
            aria-label={t("lub.license.paste_the_license_from_your")}
            value={token}
            onChange={(event) => setToken(event.target.value)}
          />
          <button
            type="button"
            className="lub-btn lub-btn-primary"
            onClick={() => {
              const result = activateLicense(token);
              setNotice(`${result.error.code} · ${prompt}`);
            }}
          >
            {t("lub.license.activate")}
          </button>
        </div>
      </div>
      {notice ? (
        <p className="lub-alert" role="alert">
          {notice}
        </p>
      ) : null}
      <div className="lub-card lub-pad">
        <div className="lub-status-row">
          <span>{t("lub.license.status")}</span>
          <StatusValue status={status} />
        </div>
        <div className="lub-status-row">
          <span>{t("lub.license.expires")}</span>
          <span data-testid="license-expires">
            {status.expiresAt ?? "—"}
            {status.graceUntil ? ` ${t("lub.license.day_grace")}` : ""}
          </span>
        </div>
        <div className="lub-status-row">
          <span>{t("lub.license.email")}</span>
          <span data-testid="license-email">{status.emailMasked ?? "—"}</span>
        </div>
      </div>
      <p className="lub-muted">
        {t("lub.license.no_license")}
        {proCompareEnabled ? (
          <>
            {" "}
            <a
              href="#free-vs-pro"
              className="lub-inline-link"
              onClick={(event) => event.preventDefault()}
            >
              {t("lub.license.compare_free_vs_pro")}
            </a>
          </>
        ) : null}
      </p>
    </>
  );

  if (variant === "embedded") {
    return (
      <div data-testid="license-settings" data-variant="embedded">
        {form}
      </div>
    );
  }

  return (
    <div className="lub-win" data-testid="license-settings">
      <nav className="lub-side" aria-label="License">
        {NAV.map(([key, zh, en]) => {
          const current = key === "license";
          const label =
            key === "license"
              ? "License"
              : t(key, {
                  defaultValue: localizedDefault(i18n.language, zh, en),
                });
          return (
            <button
              key={key}
              type="button"
              aria-current={current ? "page" : undefined}
              onClick={(event) => event.preventDefault()}
            >
              {label}
            </button>
          );
        })}
      </nav>
      <div className="lub-main">{form}</div>
    </div>
  );
}
