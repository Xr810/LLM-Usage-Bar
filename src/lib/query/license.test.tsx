import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import type { ReactNode } from "react";
import { describe, expect, expectTypeOf, it } from "vitest";
import { getLicenseStatus } from "@/lib/api/license";
import type { LicenseStatus } from "@/types/license";
import { server } from "../../../tests/msw/server";
import { useLicenseStatus } from "./license";

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("get_license_status", () => {
  it("types optional identity fields as omitted strings, not null", () => {
    expectTypeOf<LicenseStatus["expiresAt"]>().toEqualTypeOf<
      string | undefined
    >();
    expectTypeOf<LicenseStatus["graceUntil"]>().toEqualTypeOf<
      string | undefined
    >();
    expectTypeOf<LicenseStatus["emailMasked"]>().toEqualTypeOf<
      string | undefined
    >();
    expectTypeOf<LicenseStatus["build"]>().toEqualTypeOf<
      "community" | "official"
    >();
    expectTypeOf<
      LicenseStatus["uiFlags"]["upsellEntry"]
    >().toEqualTypeOf<boolean>();
  });

  it("returns the community payload with upsellEntry off and no empty optionals", async () => {
    const status = await getLicenseStatus();
    expect(status).toEqual({
      tier: "free",
      build: "community",
      features: [],
      uiFlags: { upsellEntry: false },
    });
    expect(status.expiresAt).toBeUndefined();
    expect(status.graceUntil).toBeUndefined();
    expect(status.emailMasked).toBeUndefined();
  });

  it("keeps a present upsellEntry flag without treating missing optionals as null", async () => {
    const official: LicenseStatus = {
      tier: "pro",
      build: "official",
      features: ["heatmap_12m"],
      uiFlags: { upsellEntry: true },
      expiresAt: "2026-12-01T00:00:00Z",
      graceUntil: "2026-12-15T00:00:00Z",
      emailMasked: "a***@example.com",
    };
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json(official),
      ),
    );
    const { result } = renderHook(() => useLicenseStatus(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(official);
    expect(result.current.data?.uiFlags.upsellEntry).toBe(true);
  });
});
