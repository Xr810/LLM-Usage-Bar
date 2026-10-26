import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
  communityLicenseStatus,
  type LicenseStatus,
} from "@/components/lub/licenseMock";

// Decided (Q-RX-4): community follows Free limits.
export const COMMUNITY_FOLLOWS_FREE_LIMITS = true;

export const licenseStatusQueryKey = ["license-status"] as const;

/** Reads {@link COMMUNITY_FOLLOWS_FREE_LIMITS}. Tests spy this to cover both values. */
export function communityFollowsFreeLimits(): boolean {
  return COMMUNITY_FOLLOWS_FREE_LIMITS;
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "";
}

/** C2 is not registered yet. A real license failure must not match this. */
export function isUnwiredLicenseCommand(error: unknown): boolean {
  const message = errorText(error);
  return (
    /command\s+get_license_status\s+not found/i.test(message) ||
    /unknown command/i.test(message) ||
    /not registered/i.test(message)
  );
}

/**
 * C2 `get_license_status`. When the command is not wired, return the community
 * mock. Any other failure rejects so callers can treat it as restricted.
 */
export async function fetchLicenseStatus(): Promise<LicenseStatus> {
  try {
    return await invoke<LicenseStatus>("get_license_status");
  } catch (error) {
    if (isUnwiredLicenseCommand(error)) return communityLicenseStatus();
    throw error instanceof Error ? error : new Error(errorText(error));
  }
}

export interface LicenseStatusResult {
  data: LicenseStatus | undefined;
  isPending: boolean;
  isLoading: boolean;
  isError: boolean;
}

export function useLicenseStatus(): LicenseStatusResult {
  const query = useQuery({
    queryKey: licenseStatusQueryKey,
    queryFn: fetchLicenseStatus,
    retry: false,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  });
  return {
    data: query.data,
    isPending: query.isPending,
    isLoading: query.isLoading,
    isError: query.isError,
  };
}

/**
 * Restricted dashboard: official Free, or a community build while
 * {@link COMMUNITY_FOLLOWS_FREE_LIMITS} is on. Loading and errors pass
 * `null` / `undefined` and are restricted.
 */
export function isDashboardLimited(
  status: LicenseStatus | null | undefined,
  communityFollows: boolean = COMMUNITY_FOLLOWS_FREE_LIMITS,
): boolean {
  if (status == null) return true;
  const isOfficialFree = status.build === "official" && status.tier === "free";
  const communityLimited = status.build === "community" && communityFollows;
  return isOfficialFree || communityLimited;
}
