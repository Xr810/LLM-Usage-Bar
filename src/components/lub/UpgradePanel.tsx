import { useTranslation } from "react-i18next";
import { MOCK_PURCHASE_INFO } from "@/lib/api/purchase";
import type { PurchaseInfo } from "@/types/purchase";
import { ProCompareTable } from "./ProCompareTable";
import { PurchaseSlot } from "./PurchaseSlot";

/**
 * One upgrade surface: title, Free/Pro table, and a purchase slot.
 * The slot renders nothing while `PURCHASE_UI_ENABLED` is false.
 */
export function UpgradePanel({
  onClose,
  upsellEntry,
  purchase = MOCK_PURCHASE_INFO,
}: {
  onClose: () => void;
  upsellEntry: boolean;
  purchase?: PurchaseInfo;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="lub-upgrade"
      role="dialog"
      aria-labelledby="lub-upgrade-title"
      data-testid="upgrade-panel"
    >
      <h2 id="lub-upgrade-title" className="lub-title">
        {t("lub.procompare.title")}
      </h2>
      <p className="lub-muted">{t("lub.procompare.subtitle")}</p>
      <ProCompareTable />
      <p className="lub-muted">{t("lub.procompare.offline_note")}</p>
      <PurchaseSlot info={purchase} upsellEntry={upsellEntry} />
      <div className="lub-upsell-actions">
        <button type="button" className="lub-btn" onClick={onClose}>
          {t("lub.upgrade.close")}
        </button>
      </div>
    </div>
  );
}
