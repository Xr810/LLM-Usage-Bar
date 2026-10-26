/** Display hints only. These flags never authorize Pro operations. */
export interface LicenseUiFlags {
  upsellEntry: boolean;
}

/** C2 / §7. Expired Pro stays `free`; there is no extra state value. */
export type LicenseState = "active" | "grace" | "free";

/**
 * Wire payload of get_license_status / activate_license / license-changed.
 * CamelCase. `expiresAt`, `graceUntil`, and `emailMasked` are omitted when
 * empty — they are never null.
 * Community: `{tier:'free',state:'free',build:'community',features:[],uiFlags:{upsellEntry:false}}`.
 * `state: 'free'` with a non-empty `expiresAt` is an expired Pro.
 */
export interface LicenseStatus {
  tier: "free" | "pro" | "pro_p3";
  state: LicenseState;
  expiresAt?: string;
  graceUntil?: string;
  build: "community" | "official";
  features: string[];
  uiFlags: LicenseUiFlags;
  emailMasked?: string;
}

export type LicenseChangedPayload = LicenseStatus;
