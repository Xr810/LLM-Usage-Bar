import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import { usageDashboardKeys } from "@/lib/query/usageDashboard";
import { emitTauriEvent } from "../msw/tauriMocks";

describe("useUsageEventBridge", () => {
  it("treats the payload-free dashboard invalidation as a global cache event", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    renderHook(() => useUsageEventBridge(), { wrapper });
    await waitFor(() => {
      emitTauriEvent("usage-dashboard-invalidated");
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: usageDashboardKeys.all,
      });
    });
  });

  it("advances the live range before invalidating and unregisters on cleanup", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const calls: string[] = [];
    const invalidate = vi
      .spyOn(queryClient, "invalidateQueries")
      .mockImplementation(async () => {
        calls.push("invalidate");
      });
    const onUsageRecorded = vi.fn(() => calls.push("clock"));
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { unmount } = renderHook(() => useUsageEventBridge(onUsageRecorded), {
      wrapper,
    });
    await waitFor(() => {
      emitTauriEvent("usage-dashboard-invalidated");
      expect(onUsageRecorded).toHaveBeenCalledTimes(1);
    });

    expect(calls[0]).toBe("clock");
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: usageDashboardKeys.all,
    });
    unmount();
    emitTauriEvent("usage-dashboard-invalidated");
    expect(onUsageRecorded).toHaveBeenCalledTimes(1);
  });
});
