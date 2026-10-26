import type { QueryClient } from "@tanstack/react-query";
import type { LicenseStatus } from "@/types/license";
import { licenseStatusQueryKey } from "@/hooks/useLicenseStatus";
import { officialProLicenseStatus } from "./msw/handlers";

export function licenseStatus(
  tier: LicenseStatus["tier"],
  build: LicenseStatus["build"] = "official",
): LicenseStatus {
  return {
    tier,
    build,
    features: [],
    uiFlags: { upsellEntry: false },
  };
}

/** Prime the cache so Pro dashboard tests do not flash the loading-limited UI. */
export function primeOfficialProLicense(client: QueryClient) {
  client.setQueryData(licenseStatusQueryKey, officialProLicenseStatus);
}
