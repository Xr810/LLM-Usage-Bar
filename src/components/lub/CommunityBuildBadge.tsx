import { useTranslation } from "react-i18next";

/** Neutral tag. Same fill / secondary-text treatment as the future PRO tag. */
export function CommunityBuildBadge() {
  const { t } = useTranslation();
  return (
    <span className="lub-tag" aria-label={t("lub.community.aria_badge")}>
      {t("lub.community.badge")}
    </span>
  );
}
