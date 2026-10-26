import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import {
  allowlistedPurchaseUrl,
  getPurchaseInfo,
  isAllowlistedPurchaseUrl,
  loadPurchaseInfo,
  MOCK_PURCHASE_INFO,
  POLAR_CHECKOUT_HOST,
} from "./purchase";

describe("get_purchase_info", () => {
  it("keeps the mock off the unwired command", async () => {
    await expect(loadPurchaseInfo()).resolves.toEqual({
      trialAvailable: true,
      trialDays: 7,
    });
    expect(MOCK_PURCHASE_INFO.purchaseUrl).toBeUndefined();
    expect(commandCalls("get_purchase_info")).toHaveLength(0);

    const source = readFileSync("src/lib/api/purchase.ts", "utf8");
    expect(source).toContain('invoke<PurchaseInfo>("get_purchase_info")');
    expect(source).toContain("MOCK");
    expect(getPurchaseInfo).toEqual(expect.any(Function));
    expect(source).toContain("Polar.sh");
    expect(source).toContain(POLAR_CHECKOUT_HOST);
    expect(source.toLowerCase()).not.toContain("squeezy");
  });

  it("allowlists only the Polar checkout host", () => {
    const checkout = "https://buy.polar.sh/checkout/abc";
    expect(isAllowlistedPurchaseUrl(checkout)).toBe(true);
    expect(isAllowlistedPurchaseUrl("https://BUY.POLAR.SH/checkout/abc")).toBe(
      true,
    );
    expect(allowlistedPurchaseUrl(checkout)).toBe(checkout);
    for (const blocked of [
      undefined,
      "",
      "   ",
      "buy.polar.sh/checkout/abc",
      "http://buy.polar.sh/checkout/abc",
      "https://polar.sh/checkout/abc",
      "https://evil.buy.polar.sh/checkout/abc",
      "https://user:pass@buy.polar.sh/checkout/abc",
      "https://checkout.paddle.com/pay/abc",
      "https://example.com/pro",
    ]) {
      expect(isAllowlistedPurchaseUrl(blocked)).toBe(false);
      expect(allowlistedPurchaseUrl(blocked)).toBeUndefined();
    }
  });
});
