import type { ReactNode } from "react";
import {
  useQuery,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { LicenseStatus } from "@/types/license";
import { emitTauriEvent } from "../../../tests/msw/tauriMocks";
import {
  licenseKeys,
  refreshQueriesAfterLicenseChange,
  useLicenseChangedRefresh,
} from "./license";

const gracePayload: LicenseStatus = {
  tier: "pro",
  state: "grace",
  graceUntil: "2026-12-15T00:00:00Z",
  build: "official",
  features: ["heatmap_12m"],
  uiFlags: { upsellEntry: true },
};

describe("license-changed refresh", () => {
  it("refetches every cached query, including inactive ones", async () => {
    expect(gracePayload.emailMasked).toBeUndefined();
    expect(gracePayload.state).toBe("grace");
    expect(gracePayload.graceUntil).toBe("2026-12-15T00:00:00Z");

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const license = vi.fn(async () => gracePayload);
    const settings = vi.fn(async () => ({ tier: "pro" }));
    const providers = vi.fn(async () => [{ id: "openai" }]);
    const usage = vi.fn(async () => ({ requests: 1 }));
    await client.prefetchQuery({
      queryKey: ["usage", "summary"],
      queryFn: usage,
    });

    function Probe() {
      useLicenseChangedRefresh();
      useQuery({ queryKey: licenseKeys.status(), queryFn: license });
      useQuery({ queryKey: ["settings"], queryFn: settings });
      useQuery({
        queryKey: ["usage-dashboard", "providers"],
        queryFn: providers,
      });
      return null;
    }

    render(
      <QueryClientProvider client={client}>
        <Probe />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(license).toHaveBeenCalledTimes(1));
    expect(settings).toHaveBeenCalledTimes(1);
    expect(providers).toHaveBeenCalledTimes(1);
    expect(usage).toHaveBeenCalledTimes(1);

    emitTauriEvent("license-changed", gracePayload);
    await waitFor(() => expect(license).toHaveBeenCalledTimes(2));
    expect(settings).toHaveBeenCalledTimes(2);
    expect(providers).toHaveBeenCalledTimes(2);
    expect(usage).toHaveBeenCalledTimes(2);

    emitTauriEvent("license-changed", gracePayload);
    await waitFor(() => expect(settings).toHaveBeenCalledTimes(3));
    expect(license).toHaveBeenCalledTimes(3);
    expect(providers).toHaveBeenCalledTimes(3);
    expect(usage).toHaveBeenCalledTimes(3);
  });

  it("coalesces concurrent refreshes and runs one trailing refetch", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    let release: () => void = () => undefined;
    let block = true;
    const invalidate = vi
      .spyOn(client, "invalidateQueries")
      .mockImplementation(() => {
        if (!block) return Promise.resolve();
        block = false;
        return new Promise((resolve) => {
          release = () => resolve(undefined);
        });
      });

    const first = refreshQueriesAfterLicenseChange(client);
    const second = refreshQueriesAfterLicenseChange(client);
    const third = refreshQueriesAfterLicenseChange(client);
    expect(second).toBe(first);
    expect(third).toBe(first);
    expect(invalidate).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledWith({ refetchType: "all" });

    release();
    await first;
    expect(invalidate).toHaveBeenCalledTimes(2);

    await refreshQueriesAfterLicenseChange(client);
    expect(invalidate).toHaveBeenCalledTimes(3);
    invalidate.mockRestore();
  });

  it("does not refetch after the subscriber unmounts", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const settings = vi.fn(async () => ({ ok: true }));
    function Probe() {
      useLicenseChangedRefresh();
      useQuery({ queryKey: ["settings"], queryFn: settings });
      return null;
    }
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { unmount } = render(<Probe />, { wrapper });
    await waitFor(() => expect(settings).toHaveBeenCalledTimes(1));
    unmount();
    emitTauriEvent("license-changed", gracePayload);
    await Promise.resolve();
    expect(settings).toHaveBeenCalledTimes(1);
  });
});
