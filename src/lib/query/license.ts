import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { getLicenseStatus } from "@/lib/api/license";
import type { LicenseStatus } from "@/types/license";

export const licenseKeys = {
  all: ["license"] as const,
  status: () => [...licenseKeys.all, "status"] as const,
};

export function useLicenseStatus() {
  const queryClient = useQueryClient();

  useTauriEvent<LicenseStatus>("license-changed", (status) => {
    queryClient.setQueryData(licenseKeys.status(), status);
  });

  return useQuery({
    queryKey: licenseKeys.status(),
    queryFn: getLicenseStatus,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
}
