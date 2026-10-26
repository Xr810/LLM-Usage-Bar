import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import type { LicenseStatus, LicenseUiFlags } from "@/types/license";
import type { LubBuild, LubTier } from "./panelModel";

export type { LicenseStatus };

export interface LubEdition {
  build: LubBuild;
  /** BE-1b. False when `uiFlags.upsellEntry` is missing or not true. */
  upsellEntry: boolean;
}

const BUILDS = new Set<string>(["community", "official"]);

export function readLubBuild(
  value: string | null | undefined,
): LubBuild | null {
  return value != null && BUILDS.has(value) ? (value as LubBuild) : null;
}

/** Missing flags, or any value other than true, stay false. */
export function readUpsellEntry(flags: LicenseUiFlags | undefined): boolean {
  return flags?.upsellEntry === true;
}

export interface DetectLubBuildInput {
  license?: LicenseStatus | null;
  /** True while `get_license_status` has not settled, including after a failure. */
  pending?: boolean;
}

const COMMUNITY_EDITION: LubEdition = {
  build: "community",
  upsellEntry: false,
};

/**
 * Build comes only from `get_license_status.build` — the same provider the
 * tray snapshot's `buildType` is read from. Loading, a failed command, or a
 * missing/unknown build stays `community`, so a community install never
 * shows a purchase entry.
 */
export function resolveLubBuild(input: DetectLubBuildInput = {}): LubBuild {
  if (input.pending || input.license == null) return "community";
  return readLubBuild(input.license.build) ?? "community";
}

/** Reads `get_license_status`. Does not change the command contract. */
export async function detectLubBuild(): Promise<LubEdition> {
  try {
    const license = await invoke<LicenseStatus | null>("get_license_status");
    const build = readLubBuild(license?.build);
    if (license == null || build == null) return COMMUNITY_EDITION;
    return {
      build,
      upsellEntry: readUpsellEntry(license.uiFlags),
    };
  } catch {
    return COMMUNITY_EDITION;
  }
}

/**
 * Community until `get_license_status` settles. A later `license-changed`
 * refetches the provider build without resetting to community first, so a
 * duplicate delivery does not flash locked chrome. `upsellEntry` stays
 * false until a known build says true.
 */
export function useLubEdition(): LubEdition {
  const [edition, setEdition] = useState<LubEdition>(COMMUNITY_EDITION);
  const requestId = useRef(0);

  useEffect(() => {
    const id = ++requestId.current;
    let cancelled = false;
    setEdition(COMMUNITY_EDITION);
    detectLubBuild()
      .then((next) => {
        if (!cancelled && requestId.current === id) setEdition(next);
      })
      .catch(() => {
        if (!cancelled && requestId.current === id) {
          setEdition(COMMUNITY_EDITION);
        }
      });
    return () => {
      cancelled = true;
      requestId.current += 1;
    };
  }, []);

  useTauriEvent<LicenseStatus>("license-changed", () => {
    const id = ++requestId.current;
    void detectLubBuild()
      .then((next) => {
        if (requestId.current === id) setEdition(next);
      })
      .catch(() => {
        if (requestId.current === id) setEdition(COMMUNITY_EDITION);
      });
  });

  return edition;
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
