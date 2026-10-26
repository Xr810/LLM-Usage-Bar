import { useTranslation } from "react-i18next";
import { ensureProCopy } from "@/i18n/proCopy";

/** Neutral PRO tag. Same track treatment as the community badge. */
export function ProTag() {
  ensureProCopy();
  const { t } = useTranslation();
  return (
    <span className="lub-pro-tag" data-testid="pro-tag">
      {t("lub.upsell.pro_tag", { ns: "pro" })}
    </span>
  );
}
