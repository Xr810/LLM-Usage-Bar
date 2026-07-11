import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";
import { emitTauriEvent } from "../msw/tauriMocks";

describe("useUsageEventBridge", () => {
  it("invalidates v13 dashboard and event queries when usage is recorded", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    renderHook(() => useUsageEventBridge(), { wrapper });
    await waitFor(() => {
      emitTauriEvent("usage-log-recorded", { providerId: "metered-api" });
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: usageDashboardKeys.dashboards(),
      });
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.eventsAll(),
    });
  });
});
