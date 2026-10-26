import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  snapshotBuildType?: string | null;
  license?: LicenseStatus | null;
  /** True while `get_license_status` has not settled, including after a failure. */
  pending?: boolean;
}

/**
 * Snapshot `buildType` wins when it is `community` or `official`.
 * Otherwise the build is `get_license_status.build`. Loading, a failed
 * command, or a missing build stays `community`, so a community install
 * never shows a purchase entry.
 */
export function resolveLubBuild(input: DetectLubBuildInput = {}): LubBuild {
  const fromSnapshot = readLubBuild(input.snapshotBuildType);
  if (fromSnapshot) return fromSnapshot;
  if (input.pending || input.license == null) return "community";
  return readLubBuild(input.license.build) ?? "community";
}

/** Reads `get_license_status`. Does not change the command contract. */
export async function detectLubBuild(input?: {
  snapshotBuildType?: string | null;
}): Promise<LubEdition> {
  const fromSnapshot = readLubBuild(input?.snapshotBuildType);
  try {
    const license = await invoke<LicenseStatus | null>("get_license_status");
    return {
      build:
        fromSnapshot ??
        resolveLubBuild({ license, pending: false, snapshotBuildType: null }),
      upsellEntry: readUpsellEntry(license?.uiFlags),
    };
  } catch {
    return {
      build: fromSnapshot ?? "community",
      upsellEntry: false,
    };
  }
}

/**
 * Community until `get_license_status` settles, unless the snapshot already
 * carries `buildType`. `upsellEntry` stays false until the payload says true.
 */
export function useLubEdition(snapshotBuildType?: string | null): LubEdition {
  const [edition, setEdition] = useState<LubEdition>({
    build: readLubBuild(snapshotBuildType) ?? "community",
    upsellEntry: false,
  });

  useEffect(() => {
    let cancelled = false;
    setEdition({
      build: readLubBuild(snapshotBuildType) ?? "community",
      upsellEntry: false,
    });
    detectLubBuild({ snapshotBuildType })
      .then((next) => {
        if (!cancelled) setEdition(next);
      })
      .catch(() => {
        if (!cancelled) {
          setEdition({
            build: readLubBuild(snapshotBuildType) ?? "community",
            upsellEntry: false,
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [snapshotBuildType]);

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
