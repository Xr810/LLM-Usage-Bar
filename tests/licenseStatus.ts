import type { QueryClient } from "@tanstack/react-query";
import type { LicenseStatus } from "@/components/lub/licenseMock";
import { licenseStatusQueryKey } from "@/hooks/useLicenseStatus";
import { officialProLicenseStatus } from "./msw/handlers";

export function licenseStatus(
  tier: LicenseStatus["tier"],
  build: LicenseStatus["build"] = "official",
): LicenseStatus {
  return {
    tier,
    expiresAt: null,
    graceUntil: null,
    build,
    features: [],
    emailMasked: null,
  };
}

/** Prime the cache so Pro dashboard tests do not flash the loading-limited UI. */
export function primeOfficialProLicense(client: QueryClient) {
  client.setQueryData(licenseStatusQueryKey, officialProLicenseStatus);
}
