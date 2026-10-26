/** C2 / C20 shell. Community builds never activate and never write license state. */

export interface LicenseStatus {
  tier: "free" | "pro" | "pro_p3";
  expiresAt: string | null;
  graceUntil: string | null;
  build: "community" | "official";
  features: string[];
  emailMasked: string | null;
}

export interface CodedError {
  code: string;
  message: string;
}

export function communityLicenseStatus(): LicenseStatus {
  return {
    tier: "free",
    expiresAt: null,
    graceUntil: null,
    build: "community",
    features: [],
    emailMasked: null,
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
