import { useTranslation } from "react-i18next";

/**
 * Q-RX-1: the community build keeps these two controls visible and disabled.
 * They send no command.
 */
export function CommunityAllowPlaceholder() {
  const { t } = useTranslation();
  return (
    <div className="lub-community" data-testid="community-allow-placeholder">
      <p className="lub-muted">{t("lub.community.placeholder_note")}</p>
      <div className="lub-community-actions">
        <button
          type="button"
          className="lub-btn"
          aria-disabled="true"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.community.allow_once")}
        </button>
        <button
          type="button"
          className="lub-btn"
          aria-disabled="true"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.community.allow_more_today")}
        </button>
      </div>
    </div>
  );
}
