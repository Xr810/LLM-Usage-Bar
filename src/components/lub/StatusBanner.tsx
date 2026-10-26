import { useTranslation } from "react-i18next";
import { displayUsd, splitUsd } from "./money";

interface StatusBannerProps {
  capUsd: string | null;
  locale: string;
  /** Batch 1 passes false, so the compare line is not rendered. */
  proCompareEnabled: boolean;
}

/** Free / community over-cap reminder. No allow, kill, or pause actions. */
export function StatusBanner({
  capUsd,
  locale,
  proCompareEnabled,
}: StatusBannerProps) {
  const { t } = useTranslation();
  const amount = displayUsd(splitUsd(capUsd, locale, "complete").text);
  return (
    <div className="lub-banner lub-banner-warn" role="status">
      <div className="lub-banner-copy">
        <b>{t("lub.overcap.title", { amount })}</b>
        <p className="lub-muted">{t("lub.overcap.alert_only_body")}</p>
        {proCompareEnabled ? (
          <p>
            {t("lub.overcap.want_auto_stop")}{" "}
            <button type="button" className="lub-link">
              {t("lub.overcap.compare_link")}
            </button>
          </p>
        ) : null}
      </div>
    </div>
  );
}
