import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { getLicenseStatus } from "@/lib/api/license";
import type { LicenseStatus } from "@/types/license";

export const licenseKeys = {
  all: ["license"] as const,
  status: () => [...licenseKeys.all, "status"] as const,
};

export function useLicenseStatus() {
  return useQuery({
    queryKey: licenseKeys.status(),
    queryFn: getLicenseStatus,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
}

interface LicenseRefreshGate {
  rerun: boolean;
  promise: Promise<void>;
}

const licenseRefreshGates = new WeakMap<QueryClient, LicenseRefreshGate>();

/**
 * `license-changed` is at-least-once (BE-31). Community builds never emit it.
 * Concurrent deliveries share one invalidation of every cached query
 * (license, dashboards, usage, provider/model/agent views, settings) and
 * one trailing refetch if another event arrives mid-flight.
 */
export function refreshQueriesAfterLicenseChange(
  queryClient: QueryClient,
): Promise<void> {
  const existing = licenseRefreshGates.get(queryClient);
  if (existing) {
    existing.rerun = true;
    return existing.promise;
  }

  let resolveDone: () => void = () => undefined;
  let rejectDone: (error: unknown) => void = () => undefined;
  const promise = new Promise<void>((resolve, reject) => {
    resolveDone = resolve;
    rejectDone = reject;
  });
  const gate: LicenseRefreshGate = { rerun: false, promise };
  licenseRefreshGates.set(queryClient, gate);

  void (async () => {
    try {
      do {
        gate.rerun = false;
        await queryClient.invalidateQueries({ refetchType: "all" });
      } while (gate.rerun);
      resolveDone();
    } catch (error) {
      rejectDone(error);
    } finally {
      if (licenseRefreshGates.get(queryClient) === gate) {
        licenseRefreshGates.delete(queryClient);
      }
    }
  })();

  return promise;
}

/** One subscription per window. The payload is the signal; the refetch is authoritative. */
export function useLicenseChangedRefresh(): void {
  const queryClient = useQueryClient();

  useTauriEvent<LicenseStatus>("license-changed", () => {
    void refreshQueriesAfterLicenseChange(queryClient).catch(
      (error: unknown) => {
        console.error("license-changed refresh failed", error);
      },
    );
  });
}
