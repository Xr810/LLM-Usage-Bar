/** Display hints only. These flags never authorize Pro operations. */
export interface LicenseUiFlags {
  upsellEntry: boolean;
}

/** Wire payload of get_license_status / activate_license / license-changed. */
export interface LicenseStatus {
  tier: "free" | "pro" | "pro_p3";
  expiresAt?: string;
  graceUntil?: string;
  build: "community" | "official";
  features: string[];
  uiFlags: LicenseUiFlags;
  emailMasked?: string;
}

export type LicenseChangedPayload = LicenseStatus;
