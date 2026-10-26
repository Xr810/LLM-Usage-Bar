import { invoke } from "@tauri-apps/api/core";
import type { PurchaseInfo } from "@/types/purchase";

/**
 * Typed wrapper for C24 `get_purchase_info`.
 * The Rust command is not registered. Do not call this until it is.
 * Swap `loadPurchaseInfo` over to this function when the backend lands.
 */
export function getPurchaseInfo(): Promise<PurchaseInfo> {
  return invoke<PurchaseInfo>("get_purchase_info");
}

/**
 * MOCK stand-in for `get_purchase_info`.
 * No price and no checkout URL. Replace the body of `loadPurchaseInfo`
 * with `return getPurchaseInfo()` when the command exists.
 */
export const MOCK_PURCHASE_INFO: PurchaseInfo = {
  trialAvailable: true,
  trialDays: 7,
};

/** MOCK. Returns {@link MOCK_PURCHASE_INFO} and does not call Tauri. */
export function loadPurchaseInfo(): Promise<PurchaseInfo> {
  return Promise.resolve(MOCK_PURCHASE_INFO);
}
