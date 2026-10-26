import { useTranslation } from "react-i18next";
import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import { displayUsd } from "./money";

/** Design default until C2 publishes the daily allowance. */
export const DEFAULT_COMMUNITY_ALLOWANCE_USD = 5;

function AllowButton({
  label,
  unavailable,
}: {
  label: string;
  unavailable: string;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className="lub-allow-button opacity-45 cursor-not-allowed"
      aria-disabled="true"
      title={unavailable}
      aria-label={t("lub.community.aria_allow_disabled", { action: label })}
      onClick={() => {
        /* Placeholder only. A click does not send a command. */
      }}
    >
      {label}
    </button>
  );
}

/**
 * Greyed allow actions for a community build that is over its cap.
 * Focusable, but a click sends no command and offers no purchase link.
 */
export function CommunityAllowPlaceholder({
  locale,
  allowanceUsd = DEFAULT_COMMUNITY_ALLOWANCE_USD,
}: {
  locale: string;
  allowanceUsd?: number;
}) {
  const { t } = useTranslation();
  const unavailable = t("lub.errors.feature_unavailable");
  const allowOne = t("lub.community.allow_this_one");
  const allowMore = t("lub.community.allow_more_today", {
    amount: displayUsd(formatUsd(String(allowanceUsd), locale)),
  });
  return (
    <div className="lub-allow-placeholder" data-slot="community-allow">
      <div className="lub-allow-actions">
        <AllowButton label={allowOne} unavailable={unavailable} />
        <AllowButton label={allowMore} unavailable={unavailable} />
      </div>
      <p className="lub-allow-note">{t("lub.community.placeholder_note")}</p>
    </div>
  );
}
