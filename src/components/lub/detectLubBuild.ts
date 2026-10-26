import type { LubBuild, LubTier } from "./panelModel";

/**
 * C2 `get_license_state.build` is the license edition. The tray snapshot does
 * not carry it, and this tree has no community compile switch: Cargo features
 * are only `test-hooks`, Vite has no edition env, and `tauri.conf.json` names
 * one product. Reading a new snapshot field would change the backend contract.
 * Production stays official until C2 is wired. Tests pass `build` in directly.
 */
export function detectLubBuild(): LubBuild {
  return "official";
}

/**
 * Community chrome is the build itself. `upsell_entry` and tier do not show
 * or hide it, and they do not add a purchase or compare link.
 */
export function communityChromeVisible(input: {
  build: LubBuild;
  upsellEntry?: boolean;
  tier?: LubTier;
}): boolean {
  return input.build === "community";
}
