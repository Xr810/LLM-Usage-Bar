/** C2 / C20 shell. Community builds never activate and never write license state. */

import type { LicenseStatus } from "@/types/license";

export type { LicenseStatus };

export interface CodedError {
  code: string;
  message: string;
}

export function communityLicenseStatus(): LicenseStatus {
  return {
    tier: "free",
    build: "community",
    features: [],
    uiFlags: { upsellEntry: false },
  };
}

export function activateLicense(_token: string): {
  ok: false;
  error: CodedError;
} {
  return {
    ok: false,
    error: {
      code: "COMMUNITY_BUILD",
      message: "Official build required",
    },
  };
}
