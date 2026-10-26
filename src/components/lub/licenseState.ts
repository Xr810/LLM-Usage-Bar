import type { LicenseState, LicenseStatus } from "@/types/license";

/** Omitted, blank, or whitespace `expiresAt` counts as empty. */
export function expiresAtPresent(
  expiresAt: string | null | undefined,
): boolean {
  return typeof expiresAt === "string" && expiresAt.trim().length > 0;
}

/**
 * §7. `state: "free"` with a stored expiry is an expired Pro.
 * The state enum stays `active | grace | free`.
 */
export function isExpiredPro(status: {
  state?: LicenseState | null;
  expiresAt?: string | null;
}): boolean {
  return status.state === "free" && expiresAtPresent(status.expiresAt);
}

/** §7. `state: "free"` with no expiry was never activated, or was deactivated. */
export function isNormalFree(status: {
  state?: LicenseState | null;
  expiresAt?: string | null;
}): boolean {
  return status.state === "free" && !expiresAtPresent(status.expiresAt);
}

/**
 * Renewal is an upsell on an official expired Pro.
 * The purchase control itself stays off while `PURCHASE_UI_ENABLED` is false.
 */
export function renewalEntryVisible(status: LicenseStatus): boolean {
  return (
    status.build === "official" &&
    status.uiFlags.upsellEntry === true &&
    isExpiredPro(status)
  );
}
