import { useTranslation } from "react-i18next";
import { displayUsd, splitUsd } from "./money";
import { ProTag } from "./ProTag";

interface StatusBannerProps {
  capUsd: string | null;
  locale: string;
  /** C2 `uiFlags.upsellEntry`. Off hides the Free vs Pro line. */
  upsellEntry: boolean;
  /** Official Free only. The tag sits on the compare control. */
  showProTag?: boolean;
  onCompare?: () => void;
}

/** Free / community over-cap reminder. No allow, kill, or pause actions. */
export function StatusBanner({
  capUsd,
  locale,
  upsellEntry,
  showProTag = false,
  onCompare,
}: StatusBannerProps) {
  const { t } = useTranslation();
  const amount = displayUsd(splitUsd(capUsd, locale, "complete").text);
  const compare = t("lub.overcap.compare_link");
  return (
    <div className="lub-banner lub-banner-warn" role="status">
      <div className="lub-banner-copy">
        <b>{t("lub.overcap.title", { amount })}</b>
        <p className="lub-muted">{t("lub.overcap.alert_only_body")}</p>
        {upsellEntry ? (
          <p>
            {t("lub.overcap.want_auto_stop")}{" "}
            <button
              type="button"
              className="lub-link"
              aria-label={compare}
              onClick={onCompare}
            >
              {compare}
              {showProTag ? <ProTag /> : null}
            </button>
          </p>
        ) : null}
      </div>
    </div>
  );
}
