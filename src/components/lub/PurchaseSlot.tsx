import { useTranslation } from "react-i18next";
import { allowlistedPurchaseUrl } from "@/lib/api/purchase";
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
  purpose = "buy",
}: {
  info: PurchaseInfo;
  upsellEntry: boolean;
  enabled?: boolean;
  /** Renewal uses the same hidden purchase slot. */
  purpose?: "buy" | "renew";
}) {
  const { t } = useTranslation();
  const showTrial =
    enabled && upsellEntry && info.trialAvailable && info.trialDays > 0;
  const purchaseUrl = enabled
    ? allowlistedPurchaseUrl(info.purchaseUrl)
    : undefined;
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
          {t(purpose === "renew" ? "lub.license.renew" : "lub.purchase.buy")}
        </button>
      ) : null}
    </div>
  );
}
