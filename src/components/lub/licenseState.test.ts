import { describe, expect, expectTypeOf, it } from "vitest";
import type { LicenseState, LicenseStatus } from "@/types/license";
import {
  expiresAtPresent,
  isExpiredPro,
  isNormalFree,
  renewalEntryVisible,
} from "./licenseState";

function status(
  partial: Partial<LicenseStatus> & Pick<LicenseStatus, "state">,
): LicenseStatus {
  return {
    tier: "free",
    build: "official",
    features: [],
    uiFlags: { upsellEntry: true },
    ...partial,
  };
}

describe("license state §7", () => {
  it("keeps the state enum at active, grace, and free", () => {
    expectTypeOf<LicenseState>().toEqualTypeOf<"active" | "grace" | "free">();
  });

  it("treats free plus a stored expiry as expired Pro", () => {
    const expired = status({
      state: "free",
      expiresAt: "2026-01-01T00:00:00Z",
    });
    expect(expiresAtPresent(expired.expiresAt)).toBe(true);
    expect(isExpiredPro(expired)).toBe(true);
    expect(isNormalFree(expired)).toBe(false);
    expect(renewalEntryVisible(expired)).toBe(true);
  });

  it("treats free with an empty expiry as never activated or deactivated", () => {
    for (const expiresAt of [undefined, "", "   ", null] as const) {
      expect(expiresAtPresent(expiresAt)).toBe(false);
      const free = status({
        state: "free",
        ...(expiresAt == null ? {} : { expiresAt }),
      });
      expect(isExpiredPro(free)).toBe(false);
      expect(isNormalFree(free)).toBe(true);
      expect(renewalEntryVisible(free)).toBe(false);
    }
    expect(isExpiredPro({ state: "free", expiresAt: null })).toBe(false);
    expect(isNormalFree({ state: "free" })).toBe(true);
  });

  it("does not mark active or grace licenses expired", () => {
    for (const state of ["active", "grace"] as const) {
      const current = status({
        state,
        tier: "pro",
        expiresAt: "2026-12-01T00:00:00Z",
      });
      expect(isExpiredPro(current)).toBe(false);
      expect(isNormalFree(current)).toBe(false);
      expect(renewalEntryVisible(current)).toBe(false);
    }
  });

  it("gates the renewal entry on an official upsell", () => {
    const expired = status({
      state: "free",
      expiresAt: "2026-01-01T00:00:00Z",
      uiFlags: { upsellEntry: false },
    });
    expect(isExpiredPro(expired)).toBe(true);
    expect(renewalEntryVisible(expired)).toBe(false);
    expect(
      renewalEntryVisible({
        ...expired,
        build: "community",
        uiFlags: { upsellEntry: true },
      }),
    ).toBe(false);
    expect(
      renewalEntryVisible({
        ...expired,
        uiFlags: { upsellEntry: true },
      }),
    ).toBe(true);
  });
});
