/** Display hints only. These flags never authorize Pro operations. */
export interface LicenseUiFlags {
  upsellEntry: boolean;
}

/**
 * `active | grace | free`. Same snake_case values as `LicenseState` in
 * `src-tauri/src/policy/license.rs` (BE-31 / #114).
 * Expired Pro stays `free`; there is no extra state value.
 */
export type LicenseState = "active" | "grace" | "free";

/**
 * Wire payload of get_license_status / activate_license / license-changed.
 * CamelCase field names match `LicenseStatus` in `policy/license.rs`:
 * `state`, `expiresAt`, `graceUntil`, `graceDays`, `uiFlags`, `emailMasked`.
 * `state` is always present. The optional instants and `graceDays` are
 * omitted when empty — they are never null. `license-changed` carries this
 * full status (including `state` and `graceUntil` when grace applies) and
 * omits `emailMasked` when the provider has none.
 * Community: `{tier:'free',state:'free',build:'community',features:[],uiFlags:{upsellEntry:false}}`.
 * `state: 'free'` with a non-empty `expiresAt` is an expired Pro.
 */
export interface LicenseStatus {
  tier: "free" | "pro" | "pro_p3";
  state: LicenseState;
  expiresAt?: string;
  graceUntil?: string;
  graceDays?: number;
  build: "community" | "official";
  features: string[];
  uiFlags: LicenseUiFlags;
  emailMasked?: string;
}

export type LicenseChangedPayload = LicenseStatus;
