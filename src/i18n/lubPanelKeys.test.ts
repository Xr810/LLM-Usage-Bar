import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
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

describe("lub panel copy", () => {
  it("matches the v5.8 asset pack and has no lub keys outside it", () => {
    expect(flat(zh.lub).sort()).toEqual(flat(packLub("zh-CN.json")).sort());
    expect(flat(en.lub).sort()).toEqual(flat(packLub("en.json")).sort());
    expect(flat(proZh.lub).sort()).toEqual(
      flat(packLub("pro.zh-CN.json")).sort(),
    );
    expect(flat(proEn.lub).sort()).toEqual(flat(packLub("pro.en.json")).sort());
    // The uploaded pack is 203 public keys: the v5.8 set of 199 plus
    // community badge/aria keys and errors.feature_unavailable.
    expect(flat(zh.lub)).toHaveLength(203);
    expect(flat(en.lub)).toHaveLength(203);
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
  });

  it("leaves zh-TW and ja on the English fallback for lub keys", () => {
    expect(zhTW).not.toHaveProperty("lub");
    expect(ja).not.toHaveProperty("lub");
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
