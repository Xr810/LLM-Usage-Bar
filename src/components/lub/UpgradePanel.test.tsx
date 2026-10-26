import { readFileSync } from "node:fs";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { settingsApi } from "@/lib/api/settings";
import type { PurchaseInfo } from "@/types/purchase";
import { designPanelFixture } from "./panelFixture";
import { compareModelOmitsPrices } from "./proCompareModel";
import { PURCHASE_UI_ENABLED, PurchaseSlot } from "./PurchaseSlot";
import { TrayPanelV5 } from "./TrayPanelV5";
import { UpgradePanel } from "./UpgradePanel";

const readyPurchase: PurchaseInfo = {
  trialAvailable: true,
  trialDays: 7,
  purchaseUrl: "https://buy.polar.sh/checkout/test",
};

const hiddenPriceCopy = [
  zh.lub.procompare.trial,
  zh.lub.procompare.refund,
  zh.lub.procompare.early_bird_founder,
  zh.lub.procompare.beta_free,
  zh.lub.procompare.p3_price,
  zh.lub.procompare.p3_only_with_pro,
  zh.lub.procompare.start_trial,
  zh.lub.purchase.buy,
  "开始 7 天试用",
  "开始 7 天免费试用",
];

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

function renderPanel(purchase: PurchaseInfo = readyPurchase) {
  return render(
    <UpgradePanel upsellEntry purchase={purchase} onClose={() => undefined} />,
  );
}

describe("UpgradePanel", () => {
  it("shows the Free/Pro table and hides price, trial, and purchase", () => {
    expect(PURCHASE_UI_ENABLED).toBe(false);
    expect(compareModelOmitsPrices()).toBe(true);
    renderPanel();
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent(zh.lub.procompare.title);
    expect(dialog).toHaveTextContent(zh.lub.procompare.subtitle);
    expect(
      within(dialog).getByRole("table", { name: zh.lub.upgrade.aria_table }),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(zh.lub.procompare.group_see),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(zh.lub.procompare.caps_free),
    ).toBeInTheDocument();
    expect(
      within(dialog).getAllByText(zh.lub.procompare.coming_soon).length,
    ).toBeGreaterThan(0);
    expect(within(dialog).getAllByText("Free").length).toBeGreaterThan(0);
    expect(within(dialog).getAllByText("Pro").length).toBeGreaterThan(0);
    const slot = dialog.querySelector("[data-slot='purchase']");
    expect(slot).toHaveAttribute("hidden");
    expect(slot).toHaveAttribute("data-purchase-ui", "off");
    expect(slot).toHaveTextContent("");
    for (const copy of hiddenPriceCopy) {
      expect(dialog).not.toHaveTextContent(copy);
    }
    expect(dialog.querySelector("a, [href]")).toBeNull();
    expect(
      screen.queryByRole("button", { name: zh.lub.purchase.buy }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /试用|trial/i })).toBeNull();

    const table = readFileSync(
      "src/components/lub/ProCompareTable.tsx",
      "utf8",
    );
    expect(table).not.toMatch(/price_|start_trial|beta_free|p3_price|\$[0-9]/);
    const panel = readFileSync("src/components/lub/UpgradePanel.tsx", "utf8");
    expect(panel).not.toMatch(/getPurchaseInfo|get_purchase_info|invoke\s*\(/);
  });

  it("still renders nothing from the slot when trial and a URL are both ready", async () => {
    const user = userEvent.setup();
    const openExternal = vi
      .spyOn(settingsApi, "openExternal")
      .mockResolvedValue(undefined);
    renderPanel(readyPurchase);
    expect(screen.queryByText(zh.lub.purchase.buy)).toBeNull();
    expect(screen.queryByText("开始 7 天试用")).toBeNull();
    expect(screen.queryByText("开始 7 天免费试用")).toBeNull();
    expect(document.querySelector("a, [href]")).toBeNull();
    expect(openExternal).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: zh.lub.upgrade.close }),
    );
    expect(openExternal).not.toHaveBeenCalled();
  });

  it("keeps the trial and purchase controls behind the flag", async () => {
    const user = userEvent.setup();
    const openExternal = vi
      .spyOn(settingsApi, "openExternal")
      .mockResolvedValue(undefined);
    const { rerender } = render(
      <PurchaseSlot info={readyPurchase} upsellEntry enabled={false} />,
    );
    expect(
      screen.queryByRole("button", { name: zh.lub.purchase.buy }),
    ).toBeNull();
    expect(screen.queryByText("开始 7 天试用")).toBeNull();

    rerender(<PurchaseSlot info={readyPurchase} upsellEntry enabled />);
    expect(
      screen.getByRole("button", { name: "开始 7 天试用" }),
    ).toBeInTheDocument();
    expect(screen.queryByText(zh.lub.procompare.beta_free)).toBeNull();
    expect(screen.queryByText(zh.lub.procompare.price_month)).toBeNull();
    expect(document.querySelector("a, [href]")).toBeNull();
    await user.click(screen.getByRole("button", { name: zh.lub.purchase.buy }));
    expect(openExternal).toHaveBeenCalledWith(readyPurchase.purchaseUrl);

    rerender(
      <PurchaseSlot
        info={{ trialAvailable: false, trialDays: 7, purchaseUrl: undefined }}
        upsellEntry
        enabled
      />,
    );
    expect(screen.queryByText("开始 7 天试用")).toBeNull();
    expect(
      screen.queryByRole("button", { name: zh.lub.purchase.buy }),
    ).toBeNull();

    rerender(
      <PurchaseSlot
        info={{
          trialAvailable: false,
          trialDays: 0,
          purchaseUrl: "https://example.com/pro",
        }}
        upsellEntry
        enabled
        purpose="renew"
      />,
    );
    expect(
      screen.queryByRole("button", { name: zh.lub.license.renew }),
    ).toBeNull();

    rerender(
      <PurchaseSlot info={readyPurchase} upsellEntry enabled purpose="renew" />,
    );
    await user.click(
      screen.getByRole("button", { name: zh.lub.license.renew }),
    );
    expect(openExternal).toHaveBeenLastCalledWith(readyPurchase.purchaseUrl);
  });
});

describe("tray upgrade entry", () => {
  function over() {
    return designPanelFixture({
      state: "over",
      todayUsd: "12.00",
      dailyCapUsd: "10",
    });
  }

  it("lets official Free open the panel and keeps community allow buttons", async () => {
    const user = userEvent.setup();
    const { unmount } = render(
      <TrayPanelV5
        model={over()}
        locale="zh-CN"
        build="official"
        tier="free"
        upsellEntry
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.getByTestId("pro-tag")).toHaveTextContent("PRO");
    expect(document.querySelector("[data-slot='community-allow']")).toBeNull();
    await user.click(
      screen.getByRole("button", { name: zh.lub.overcap.compare_link }),
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent(zh.lub.procompare.group_bill);
    expect(dialog.querySelector("[data-slot='purchase']")).toHaveTextContent(
      "",
    );
    expect(screen.queryByText(zh.lub.procompare.start_trial)).toBeNull();
    expect(screen.queryByText(zh.lub.purchase.buy)).toBeNull();
    unmount();

    render(
      <TrayPanelV5
        model={over()}
        locale="zh-CN"
        build="community"
        tier="free"
        upsellEntry
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.queryByTestId("pro-tag")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
    expect(
      document.querySelectorAll("[data-slot='community-allow'] button"),
    ).toHaveLength(2);
  });

  it("hides the entry when upsellEntry is off and skips the tag for Pro", () => {
    const { unmount } = render(
      <TrayPanelV5
        model={over()}
        locale="zh-CN"
        build="official"
        tier="free"
        upsellEntry={false}
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.queryByTestId("pro-tag")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
    unmount();

    render(
      <TrayPanelV5
        model={over()}
        locale="zh-CN"
        build="official"
        tier="pro"
        upsellEntry
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.queryByTestId("pro-tag")).toBeNull();
    expect(
      screen.getByRole("button", { name: zh.lub.overcap.compare_link }),
    ).toBeInTheDocument();
  });
});
