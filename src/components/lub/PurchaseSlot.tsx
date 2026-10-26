import { useTranslation } from "react-i18next";
import { settingsApi } from "@/lib/api/settings";
import type { PurchaseInfo } from "@/types/purchase";

/**
 * Price, trial, and purchase stay unmounted until a later batch turns this on.
 * Flipping it does not add price copy. The trial label uses `{{days}}` from
 * `get_purchase_info`, and the purchase control opens `purchaseUrl` externally.
 */
export const PURCHASE_UI_ENABLED = false;

export function PurchaseSlot({
  info,
  upsellEntry,
  enabled = PURCHASE_UI_ENABLED,
}: {
  info: PurchaseInfo;
  upsellEntry: boolean;
  enabled?: boolean;
}) {
  const { t } = useTranslation();
  const showTrial =
    enabled && upsellEntry && info.trialAvailable && info.trialDays > 0;
  const purchaseUrl = enabled ? info.purchaseUrl : undefined;
  if (!showTrial && !purchaseUrl) {
    return <div data-slot="purchase" hidden data-purchase-ui="off" />;
  }
  return (
    <div
      className="lub-upsell-actions"
      data-slot="purchase"
      data-purchase-ui="on"
    >
      {showTrial ? (
        <button
          type="button"
          className="lub-btn lub-btn-primary"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.purchase.trial", { days: info.trialDays })}
        </button>
      ) : null}
      {purchaseUrl ? (
        <button
          type="button"
          className="lub-btn"
          onClick={(event) => {
            event.preventDefault();
            void settingsApi.openExternal(purchaseUrl);
          }}
        >
          {t("lub.purchase.buy")}
        </button>
      ) : null}
    </div>
  );
}
