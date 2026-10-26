import { invoke } from "@tauri-apps/api/core";
import type { PurchaseInfo } from "@/types/purchase";

/**
 * Typed wrapper for C24 `get_purchase_info`.
 * Payments use Polar.sh. Paddle is the fallback provider and is not on the
 * allowlist in this draft. The Rust command is not registered.
 * Swap `loadPurchaseInfo` over to this function when the backend lands.
 */
export function getPurchaseInfo(): Promise<PurchaseInfo> {
  return invoke<PurchaseInfo>("get_purchase_info");
}

/** Polar.sh checkout host. Draft allowlist; Paddle hosts are not included. */
export const POLAR_CHECKOUT_HOST = "buy.polar.sh";

/**
 * `purchaseUrl` may be opened only when it is https on the Polar checkout host.
 * Missing, relative, and other hosts are rejected.
 */
export function isAllowlistedPurchaseUrl(
  url: string | null | undefined,
): url is string {
  if (url == null || url.trim() === "") return false;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== "https:") return false;
  if (parsed.username !== "" || parsed.password !== "") return false;
  return parsed.hostname.toLowerCase() === POLAR_CHECKOUT_HOST;
}

/** Allowlisted URL, or undefined when the value must not be opened. */
export function allowlistedPurchaseUrl(
  url: string | null | undefined,
): string | undefined {
  return isAllowlistedPurchaseUrl(url) ? url : undefined;
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
