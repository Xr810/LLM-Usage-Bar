import type { LubBuild, LubTier } from "./panelModel";

export interface UpgradeGateInput {
  build?: LubBuild | null;
  tier?: LubTier | null;
  /** C2 `uiFlags.upsellEntry`. Missing is off. */
  upsellEntry?: boolean | null;
  /** `get_license_status` has not settled. Treated as a community build. */
  pending?: boolean;
  /** `get_license_status` failed. Treated as a community build. */
  failed?: boolean;
}

function officialUpsell(input: UpgradeGateInput): boolean {
  if (input.pending || input.failed) return false;
  return input.build === "official" && input.upsellEntry === true;
}

/**
 * Compare link and upgrade panel.
 * Community builds never show an upsell, including when the flag is true.
 * Loading and failure stay closed.
 */
export function upgradeAffordanceVisible(input: UpgradeGateInput): boolean {
  return officialUpsell(input);
}

/** PRO tag. Official Free with `upsellEntry` only. */
export function proTagVisible(input: UpgradeGateInput): boolean {
  return officialUpsell(input) && input.tier === "free";
}
