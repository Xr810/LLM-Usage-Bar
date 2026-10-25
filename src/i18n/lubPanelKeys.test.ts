import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";
import proEn from "./locales/pro.en.json";
import proZh from "./locales/pro.zh-CN.json";

function flat(value: unknown, prefix = ""): string[] {
  if (!value || typeof value !== "object") return [prefix];
  return Object.entries(value).flatMap(([key, child]) =>
    flat(child, prefix ? `${prefix}.${key}` : key),
  );
}

describe("lub panel copy", () => {
  it("keeps zh and en panel keys identical", () => {
    expect(flat(zh.lub)).toEqual(flat(en.lub));
    expect(flat(zh.lub.panel)).toHaveLength(44);
  });

  it("leaves zh-TW and ja on the English fallback for lub keys", () => {
    expect(zhTW).not.toHaveProperty("lub");
    expect(ja).not.toHaveProperty("lub");
  });

  it("keeps the 72 Pro keys in separate files and out of the public bundle", () => {
    expect(flat(proZh.lub)).toHaveLength(72);
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
