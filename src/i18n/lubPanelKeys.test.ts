import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";
import proEn from "./locales/pro.en.json";
import proZh from "./locales/pro.zh-CN.json";

const packDir = path.resolve(process.cwd(), "design/assets-v58");

function flat(value: unknown, prefix = ""): string[] {
  if (!value || typeof value !== "object") return [prefix];
  return Object.entries(value).flatMap(([key, child]) =>
    flat(child, prefix ? `${prefix}.${key}` : key),
  );
}

function packLub(file: string): unknown {
  const json = JSON.parse(readFileSync(path.join(packDir, file), "utf8")) as {
    lub: unknown;
  };
  return json.lub;
}

function dollarAmounts(value: unknown): string[] {
  if (!value || typeof value !== "object") {
    return typeof value === "string" && /\$[0-9]/.test(value) ? [value] : [];
  }
  return Object.values(value).flatMap((child) => dollarAmounts(child));
}

const LICENSE_KEYS = [
  "settings.nav.license",
  "settings.license.title",
  "settings.license.build",
  "settings.license.tier",
  "settings.license.activate",
  "settings.license.deactivate",
  "settings.license.communityNote",
];

const supplementalLicense = [
  "license.community_build",
  "license.official_build_required",
  "license.pro_expired",
  "license.renew",
] as const;

const supplementalOnboard = [
  "onboard.terms_risk",
  "onboard.claude_subscription_via_proxy",
] as const;

const supplementalUpgrade = [
  "upgrade.close",
  "upgrade.aria_table",
  "purchase.trial",
  "purchase.buy",
] as const;

function withoutLicense(keys: string[]): string[] {
  const extra = new Set<string>([
    ...LICENSE_KEYS,
    ...supplementalLicense,
    ...supplementalOnboard,
    ...supplementalUpgrade,
  ]);
  return keys.filter((key) => !extra.has(key));
}

describe("lub panel copy", () => {
  it("matches the v5.8 asset pack and has no lub keys outside it", () => {
    expect(withoutLicense(flat(zh.lub)).sort()).toEqual(
      flat(packLub("zh-CN.json")).sort(),
    );
    expect(withoutLicense(flat(en.lub)).sort()).toEqual(
      flat(packLub("en.json")).sort(),
    );
    expect(flat(proZh.lub).sort()).toEqual(
      flat(packLub("pro.zh-CN.json")).sort(),
    );
    expect(flat(proEn.lub).sort()).toEqual(flat(packLub("pro.en.json")).sort());
    // The uploaded pack is 203 public keys: the v5.8 set of 199 plus
    // community badge/aria keys and errors.feature_unavailable.
    // FE-1a adds the seven license settings keys on top of that pack.
    expect(flat(zh.lub)).toHaveLength(
      203 +
        LICENSE_KEYS.length +
        supplementalLicense.length +
        supplementalOnboard.length +
        supplementalUpgrade.length,
    );
    expect(flat(en.lub)).toHaveLength(
      203 +
        LICENSE_KEYS.length +
        supplementalLicense.length +
        supplementalOnboard.length +
        supplementalUpgrade.length,
    );
    expect(flat(proZh.lub)).toHaveLength(85);
    expect(flat(proEn.lub)).toHaveLength(85);
  });

  it("drops invented keys in favor of the pack names", () => {
    const keys = new Set(flat(zh.lub));
    for (const invented of [
      "overcap.over_cap",
      "overcap.notify_body",
      "log.empty",
      "log.retention_free",
      "log.retention_pro",
      "community.allow_once",
      "procompare.price_later",
      "procompare.free_forever",
      "procompare.group_p3",
      "procompare.weeks_52",
      "procompare.row_history",
      "procompare.row_log",
      "procompare.caps_alert_only",
      "procompare.beta",
      "procompare.row_p3",
    ]) {
      expect(keys.has(invented)).toBe(false);
    }
    expect(keys.has("overcap.status_over_cap")).toBe(true);
    expect(keys.has("overcap.notify_body_free")).toBe(true);
    expect(keys.has("overcap.notify_body_community")).toBe(true);
    expect(keys.has("log.empty_title")).toBe(true);
    expect(keys.has("log.empty_body")).toBe(true);
    expect(keys.has("log.free_note")).toBe(true);
    expect(keys.has("log.pro_note")).toBe(true);
    expect(keys.has("log.cleared_before")).toBe(true);
    expect(keys.has("community.allow_this_one")).toBe(true);
    expect(keys.has("community.badge")).toBe(true);
    expect(keys.has("community.aria_badge")).toBe(true);
    expect(keys.has("community.aria_allow_disabled")).toBe(true);
    expect(keys.has("community.placeholder_note")).toBe(true);
    expect(keys.has("errors.feature_unavailable")).toBe(true);
    expect(keys.has("panel.group_unattributed")).toBe(true);
    expect(keys.has("panel.invalid_cost_rows")).toBe(true);
    expect(keys.has("panel.quota_stale")).toBe(true);
    expect(keys.has("panel.quota_read_failed")).toBe(true);
  });

  it("keeps amount and time placeholders, with no hardcoded prices", () => {
    expect(zh.lub.overcap.notify_title).toContain("{{amount}}");
    expect(en.lub.overcap.notify_title).toContain("{{amount}}");
    expect(zh.lub.overcap.caption_resets).toContain("{{time}}");
    expect(en.lub.overcap.caption_resets).toContain("{{time}}");
    expect(zh.lub.community.allow_more_today).toContain("{{amount}}");
    expect(en.lub.community.allow_more_today).toContain("{{amount}}");
    expect(zh.lub.community.aria_allow_disabled).toContain("{{action}}");
    expect(en.lub.community.aria_allow_disabled).toContain("{{action}}");
    expect(Object.keys(zh.lub.community).sort()).toEqual(
      Object.keys(en.lub.community).sort(),
    );
    expect(Object.keys(zh.lub.errors).sort()).toEqual(
      Object.keys(en.lub.errors).sort(),
    );
    expect(zh.lub.community.badge).toBe("社区构建");
    expect(en.lub.community.badge).toBe("Community build");
    expect(zh.lub.errors.feature_unavailable).toBe("社区构建不包含此功能");
    expect(en.lub.errors.feature_unavailable).toBe(
      "Not included in the community build",
    );
    expect(dollarAmounts(zh.lub)).toEqual([]);
    expect(dollarAmounts(en.lub)).toEqual([]);
    expect(dollarAmounts(proZh.lub)).toEqual([]);
    expect(dollarAmounts(proEn.lub)).toEqual([]);
    expect(proZh.lub.capped.hit_amt_extra_was_allowed).toContain("{{amount}}");
    expect(proEn.lub.capped.hit_amt_extra_was_allowed).toContain("{{amount}}");
    expect(proZh.lub.export.total_amt_matches_attribution_api).toContain(
      "{{amount}}",
    );
    expect(proEn.lub.export.total_amt_matches_attribution_api).toContain(
      "{{amount}}",
    );
    expect(proZh.lub.hardstop.no_limit_on_allowing_each).toContain(
      "{{seconds}}",
    );
    expect(proEn.lub.hardstop.no_limit_on_allowing_each).toContain(
      "{{seconds}}",
    );
    expect(proZh.lub.identity.auto_denies_after_s_undo).toContain(
      "{{seconds}}",
    );
    expect(proEn.lub.identity.auto_denies_after_s_undo).toContain(
      "{{seconds}}",
    );
    expect(proZh.lub.hardstop.no_limit_on_allowing_each).not.toMatch(/60\s*秒/);
    expect(proEn.lub.hardstop.no_limit_on_allowing_each).not.toMatch(/60s/);
    expect(proZh.lub.identity.auto_denies_after_s_undo).not.toMatch(/60\s*秒/);
    expect(proEn.lub.identity.auto_denies_after_s_undo).not.toMatch(/60 s/);
    expect(zhTW.usageDashboard.invalidQuotaInterval).toContain("60 秒");
    expect(ja.usageDashboard.invalidQuotaInterval).toContain("60 秒");
    expect(zh.lub.purchase.trial).toContain("{{days}}");
    expect(en.lub.purchase.trial).toContain("{{days}}");
    expect(zhTW.lub.upgrade.close).not.toBe(en.lub.upgrade.close);
    expect(ja.lub.purchase.buy).not.toBe(en.lub.purchase.buy);
  });

  it("covers the lub copy in zh-TW and ja instead of falling back to English", async () => {
    expect(flat(zhTW.lub).sort()).toEqual(flat(en.lub).sort());
    expect(flat(ja.lub).sort()).toEqual(flat(en.lub).sort());
    for (const key of LICENSE_KEYS) {
      expect(flat(zh.lub)).toContain(key);
      expect(flat(en.lub)).toContain(key);
      expect(flat(zhTW.lub)).toContain(key);
      expect(flat(ja.lub)).toContain(key);
    }
    expect(zh.lub.settings.nav.license).toBe("许可证");
    expect(zh.lub.settings.license.title).toBe("许可证");
    expect(zh.lub.settings.license.build).toBe("构建");
    expect(zh.lub.settings.license.tier).toBe("方案");
    expect(zh.lub.settings.license.activate).toBe("激活");
    expect(zh.lub.settings.license.deactivate).toBe("停用");
    expect(zh.lub.settings.license.communityNote).toContain("没有购买入口");
    expect(en.lub.settings.nav.license).toBe("License");
    expect(en.lub.settings.license.title).toBe("License");
    expect(en.lub.settings.license.build).toBe("Build");
    expect(en.lub.settings.license.tier).toBe("Plan");
    expect(en.lub.settings.license.activate).toBe("Activate");
    expect(en.lub.settings.license.deactivate).toBe("Deactivate");
    expect(en.lub.settings.license.communityNote).toContain(
      "no purchase entry",
    );
    expect(zhTW.lub.settings.nav.license).toBe("授權");
    expect(ja.lub.settings.nav.license).toBe("ライセンス");

    const local = i18n.createInstance();
    await local.init({
      lng: "zh-TW",
      fallbackLng: "en",
      resources: {
        en: { translation: en },
        "zh-TW": { translation: zhTW },
        ja: { translation: ja },
      },
      interpolation: { escapeValue: false },
    });
    expect(local.t("lub.settings.about")).toBe(zhTW.lub.settings.about);
    expect(local.t("lub.panel.trends")).toBe(zhTW.lub.panel.trends);
    expect(zhTW.lub.settings.about).not.toBe(en.lub.settings.about);
    expect(zhTW.lub.panel.trends).not.toBe(en.lub.panel.trends);
    expect(local.t("lub.settings.nav.license")).toBe(
      zhTW.lub.settings.nav.license,
    );
    await local.changeLanguage("ja");
    expect(local.t("lub.settings.about")).toBe(ja.lub.settings.about);
    expect(local.t("lub.panel.trends")).toBe(ja.lub.panel.trends);
    expect(ja.lub.settings.about).not.toBe(en.lub.settings.about);
    expect(ja.lub.panel.trends).not.toBe(en.lub.panel.trends);
    expect(local.t("lub.settings.license.communityNote")).toBe(
      ja.lub.settings.license.communityNote,
    );
  });

  it("keeps the license strings in zh, en, zh-TW, and ja", () => {
    expect(zh.lub.license.community_build).toBe(zh.lub.community.badge);
    expect(en.lub.license.community_build).toBe(en.lub.community.badge);
    expect(zh.lub.license.community_build).toBe("社区构建");
    expect(en.lub.license.community_build).toBe("Community build");
    expect(zhTW.lub.license.community_build).toBe("社群組建");
    expect(ja.lub.license.community_build).toBe("コミュニティビルド");
    expect(zh.lub.license.official_build_required).toBe("需官方构建");
    expect(en.lub.license.official_build_required).toBe(
      "Official build required",
    );
    expect(zhTW.lub.license.official_build_required).toBe("需官方組建");
    expect(ja.lub.license.official_build_required).toBe("公式ビルドが必要です");
    expect(zh.lub.license.pro_expired).toBe("Pro 已过期");
    expect(en.lub.license.pro_expired).toBe("Pro expired");
    expect(zhTW.lub.license.pro_expired).toBe("Pro 已過期");
    expect(ja.lub.license.pro_expired).toBe("Pro は期限切れです");
    expect(zh.lub.license.renew).toBe("续订 Pro");
    expect(en.lub.license.renew).toBe("Renew Pro");
    expect(zhTW.lub.license.renew).toBe("續訂 Pro");
    expect(ja.lub.license.renew).toBe("Pro を更新");
    expect(zhTW.lub.license.community_build).not.toBe(
      en.lub.license.community_build,
    );
    expect(ja.lub.license.official_build_required).not.toBe(
      en.lub.license.official_build_required,
    );
  });

  it("keeps the onboard strings in zh, en, zh-TW, and ja", () => {
    const terms = {
      zh: "可能违反服务商条款，账号可能被限制，由你自行判断。",
      en: "This may conflict with provider terms. Your account may be restricted. You decide.",
      "zh-TW": "可能違反服務商條款，帳號可能被限制，由你自行判斷。",
      ja: "プロバイダの利用規約に抵触する可能性があり、アカウントが制限されることがあります。判断はご自身で行ってください。",
    };
    const proxy = {
      zh: "Claude 订阅经代理",
      en: "Claude subscription via proxy",
      "zh-TW": "Claude 訂閱經代理",
      ja: "プロキシ経由の Claude サブスクリプション",
    };
    expect(zh.lub.onboard.terms_risk).toBe(terms.zh);
    expect(en.lub.onboard.terms_risk).toBe(terms.en);
    expect(zhTW.lub.onboard.terms_risk).toBe(terms["zh-TW"]);
    expect(ja.lub.onboard.terms_risk).toBe(terms.ja);
    expect(zh.lub.onboard.claude_subscription_via_proxy).toBe(proxy.zh);
    expect(en.lub.onboard.claude_subscription_via_proxy).toBe(proxy.en);
    expect(zhTW.lub.onboard.claude_subscription_via_proxy).toBe(proxy["zh-TW"]);
    expect(ja.lub.onboard.claude_subscription_via_proxy).toBe(proxy.ja);
    expect(terms["zh-TW"]).not.toBe(terms.en);
    expect(terms.ja).not.toBe(terms.en);
  });

  it("keeps Pro keys in separate files and out of the public bundle", () => {
    expect(flat(proEn.lub)).toEqual(flat(proZh.lub));
    const overlap = flat(proZh.lub).filter((key) => flat(zh.lub).includes(key));
    expect(overlap).toEqual([]);
    const index = readFileSync(
      path.resolve(process.cwd(), "src/i18n/index.ts"),
      "utf8",
    );
    expect(index).not.toContain("pro.en");
    expect(index).not.toContain("pro.zh");
  });
});
