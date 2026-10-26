import { createInstance } from "i18next";
import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import zh from "./locales/zh.json";
import { formatLubAmount } from "./formatAmount";

describe("formatLubAmount", () => {
  it("formats USD with Intl.NumberFormat before interpolation", async () => {
    const amount = formatLubAmount(12.5, "en");
    expect(amount).toBe(
      new Intl.NumberFormat("en", {
        style: "currency",
        currency: "USD",
      }).format(12.5),
    );
    expect(amount).not.toMatch(/\{\{/);

    const i18n = createInstance();
    await i18n.init({
      lng: "en",
      fallbackLng: "en",
      resources: {
        en: { translation: en },
        zh: { translation: zh },
      },
      interpolation: { escapeValue: false },
    });

    expect(i18n.t("lub.overcap.notify_title", { amount })).toContain(amount);
    expect(i18n.t("lub.overcap.notify_title", { amount })).not.toContain(
      "{{amount}}",
    );
    expect(
      i18n.t("lub.community.allow_more_today", { amount, lng: "zh" }),
    ).toContain(amount);
    expect(i18n.t("lub.overcap.caption_resets", { time: "00:00" })).toContain(
      "00:00",
    );
  });
});
