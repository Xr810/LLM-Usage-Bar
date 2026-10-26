// node scripts/check-keys.mjs
// Exits 1 when the v5.8 asset pack disagrees with itself, or when src locales
// gain or drop a lub key. Amounts must be {{amount}} in the merged copy.
// Supplemental keys are real copy in zh, en, zh-TW, and ja, outside the pack.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packDir = path.join(root, "design/assets-v58");
const localeDir = path.join(root, "src/i18n/locales");

const flat = (o, p = "") =>
  Object.entries(o).flatMap(([k, v]) =>
    typeof v === "object" && v ? flat(v, p + k + ".") : [[p + k, v]],
  );
const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const loadPack = (file) =>
  Object.fromEntries(flat(readJson(path.join(packDir, file))));
const ph = (s) =>
  [...new Set(String(s).match(/\{\{\w+\}\}/g) || [])].sort().join();

const errs = [];
const out = [];
for (const [za, eb, tag] of [
  ["zh-CN.json", "en.json", "free"],
  ["pro.zh-CN.json", "pro.en.json", "pro"],
]) {
  const a = loadPack(za);
  const b = loadPack(eb);
  errs.push(
    ...Object.keys(a)
      .filter((k) => !(k in b))
      .map((k) => `[${tag}] missing en: ${k}`),
    ...Object.keys(b)
      .filter((k) => !(k in a))
      .map((k) => `[${tag}] missing zh-CN: ${k}`),
    ...Object.keys(a)
      .filter((k) => k in b && ph(a[k]) !== ph(b[k]))
      .map((k) => `[${tag}] placeholder: ${k}`),
  );
  out.push(`${tag} ${Object.keys(a).length}`);
}

const freePack = loadPack("zh-CN.json");
const proPack = loadPack("pro.zh-CN.json");
errs.push(
  ...Object.keys(freePack)
    .filter((k) => k in proPack)
    .map((k) => "in both free and pro: " + k),
);

function localeLub(file) {
  const json = readJson(path.join(localeDir, file));
  return Object.fromEntries(flat(json.lub ?? {}, "lub."));
}

/** Real copy that is not in the v5.8 pack yet. Required in all four locales. */
const supplementalLub = [
  "lub.license.community_build",
  "lub.license.official_build_required",
];
const fourLocales = ["zh.json", "en.json", "zh-TW.json", "ja.json"];

for (const [localeFile, packFile] of [
  ["zh.json", "zh-CN.json"],
  ["en.json", "en.json"],
  ["pro.zh-CN.json", "pro.zh-CN.json"],
  ["pro.en.json", "pro.en.json"],
]) {
  const locale = localeLub(localeFile);
  const pack = loadPack(packFile);
  for (const key of Object.keys(pack)) {
    if (!(key in locale)) errs.push(`[locale ${localeFile}] missing ${key}`);
  }
  for (const key of Object.keys(locale)) {
    const supplemental = supplementalLub.includes(key);
    if (!(key in pack) && !supplemental) {
      errs.push(`[locale ${localeFile}] extra ${key}`);
    } else if (/\$[0-9]/.test(String(locale[key]))) {
      errs.push(`[locale ${localeFile}] hardcoded amount: ${key}`);
    }
  }
}

const fourLub = Object.fromEntries(
  fourLocales.map((file) => [file, localeLub(file)]),
);
for (const key of supplementalLub) {
  if (key in freePack) errs.push(`[supplemental] already in v5.8 pack: ${key}`);
  const values = fourLocales.map((file) => fourLub[file][key]);
  fourLocales.forEach((file, index) => {
    if (values[index] == null || String(values[index]).trim() === "") {
      errs.push(`[supplemental ${file}] missing ${key}`);
    }
  });
  const placeholders = new Set(
    values.filter((value) => value != null).map((value) => ph(value)),
  );
  if (placeholders.size > 1) errs.push(`[supplemental] placeholder: ${key}`);
  if (
    fourLub["zh-TW.json"][key] != null &&
    fourLub["zh-TW.json"][key] === fourLub["en.json"][key]
  ) {
    errs.push(`[supplemental zh-TW] english fallback: ${key}`);
  }
  if (
    fourLub["ja.json"][key] != null &&
    fourLub["ja.json"][key] === fourLub["en.json"][key]
  ) {
    errs.push(`[supplemental ja] english fallback: ${key}`);
  }
}

for (const [tag, leftFile, rightFile] of [
  ["free", "zh.json", "en.json"],
  ["pro", "pro.zh-CN.json", "pro.en.json"],
]) {
  const left = localeLub(leftFile);
  const right = localeLub(rightFile);
  for (const key of Object.keys(left)) {
    if (key in right && ph(left[key]) !== ph(right[key])) {
      errs.push(`[locale ${tag}] placeholder: ${key}`);
    }
  }
}

console.log(
  errs.length
    ? errs.join("\n")
    : `OK ${out.join(" + ")} keys; supplemental ${supplementalLub.length} in zh, en, zh-TW, ja`,
);
process.exit(errs.length ? 1 : 0);
