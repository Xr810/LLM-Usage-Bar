import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import zhTW from "@/i18n/locales/zh-TW.json";

const locales = { en, ja, zh, "zh-TW": zhTW } as const;

const oldIdentityPaths = (value: unknown, path: string[] = []): string[] => {
  if (typeof value === "string") {
    return /CC Switch|cc-switch|cc_switch|ccswitch/i.test(value)
      ? [path.join(".")]
      : [];
  }
  if (!value || typeof value !== "object") return [];

  return Object.entries(value).flatMap(([key, child]) =>
    oldIdentityPaths(child, [...path, key]),
  );
};

describe("frontend identity compatibility", () => {
  it.each(Object.entries(locales))(
    "%s keeps old identity only in explicit compatibility copy",
    (_language, locale) => {
      expect(locale.providerForm).not.toHaveProperty("partnerPromotion");
      expect(locale.providerForm.legacyUpstreamPromotionLabel).toBeTruthy();

      for (const path of oldIdentityPaths(locale)) {
        expect(
          path.startsWith("providerForm.legacyUpstreamPromotion.") ||
            path === "settings.importExportHint" ||
            path === "settings.webdavSync.remoteRootDefault" ||
            path === "settings.s3Sync.remoteRootDefault",
          path,
        ).toBe(true);
      }
    },
  );

  it.each(Object.entries(locales))(
    "%s preserves the inherited promotion coupon bytes",
    (_language, locale) => {
      const promotions = locale.providerForm.legacyUpstreamPromotion;
      expect(promotions.nekocode).toContain("cc-switch");
      expect(promotions.apinebula).toContain("ccswitch");
      expect(promotions.cubence).toContain("CCSWITCH");
      expect(promotions.zetaapi).toContain("CC-SWITCH");
    },
  );
});
