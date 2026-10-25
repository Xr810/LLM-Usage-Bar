import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import { usageApi } from "@/lib/api/usage";
import { usageKeys, useRefreshOfficialPricing } from "./usage";

it("keeps the last successful freshness and count after a discarded catalog", async () => {
  const client = new QueryClient();
  client.setQueryData(usageKeys.officialPricingFreshness(), 100);
  client.setQueryData(usageKeys.officialPricingImportedCount(), 8);
  const refresh = vi
    .spyOn(usageApi, "refreshOfficialPricing")
    .mockResolvedValue({
      fetchedAt: 200,
      lastAttemptAt: 200,
      lastSuccessAt: 100,
      modelsImported: 0,
      modelsSkipped: 0,
      sourceUrl: "https://models.dev/api.json",
      batchRejected: true,
    });
  const { result, unmount } = renderHook(() => useRefreshOfficialPricing(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  try {
    await act(async () => {
      await result.current.mutateAsync();
    });
    expect(client.getQueryData(usageKeys.officialPricingFreshness())).toBe(100);
    expect(client.getQueryData(usageKeys.officialPricingImportedCount())).toBe(
      8,
    );
  } finally {
    unmount();
    client.clear();
    refresh.mockRestore();
  }
});
