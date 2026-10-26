/**
 * C24 `get_purchase_info`. CamelCase. `purchaseUrl` is omitted when empty —
 * it is never null. This payload does not authorize a purchase.
 */
export interface PurchaseInfo {
  trialAvailable: boolean;
  trialDays: number;
  purchaseUrl?: string;
}
