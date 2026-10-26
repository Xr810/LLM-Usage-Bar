import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import {
  getPurchaseInfo,
  loadPurchaseInfo,
  MOCK_PURCHASE_INFO,
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
  });
});
