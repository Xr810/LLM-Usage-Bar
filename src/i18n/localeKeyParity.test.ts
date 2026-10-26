import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";

/**
 * `zh.json` is the zh-CN bundle. The app registers it as language code `zh`.
 * en, zh, zh-TW, and ja are the four renderer locales.
 */
const bundles = [
  ["en", en],
  ["zh", zh],
  ["zh-TW", zhTW],
  ["ja", ja],
] as const;

function flatten(value: unknown, prefix = ""): Map<string, string> {
  if (typeof value === "string") {
    return new Map(prefix ? [[prefix, value]] : []);
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Unexpected locale value at ${prefix || "<root>"}`);
  }

  const entries = new Map<string, string>();
  for (const [key, child] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    for (const [childPath, childValue] of flatten(child, path)) {
      entries.set(childPath, childValue);
    }
  }
  return entries;
}

/**
 * Interpolation tokens (`{{name}}`, `{count}`) and whole ICU blocks
 * (`{count, plural, ...}` / `{var, select, ...}`). Order may follow the
 * target language; the multiset must match English.
 */
function placeholders(value: string): string[] {
  const tokens: string[] = [];
  let index = 0;
  while (index < value.length) {
    if (value.startsWith("{{", index)) {
      const end = value.indexOf("}}", index + 2);
      if (end < 0) {
        tokens.push(value.slice(index));
        break;
      }
      tokens.push(value.slice(index, end + 2));
      index = end + 2;
      continue;
    }
    if (value[index] === "{") {
      let depth = 0;
      let cursor = index;
      let closed = false;
      for (; cursor < value.length; cursor += 1) {
        if (value[cursor] === "{") depth += 1;
        else if (value[cursor] === "}") {
          depth -= 1;
          if (depth === 0) {
            tokens.push(value.slice(index, cursor + 1));
            index = cursor + 1;
            closed = true;
            break;
          }
        }
      }
      if (!closed) {
        tokens.push(value.slice(index));
        break;
      }
      continue;
    }
    index += 1;
  }
  return tokens.sort();
}

function inlineMarkup(value: string): string[] {
  return (value.match(/<\/?[^>]+>/g) ?? []).sort();
}

const english = flatten(en);

describe("locale key parity", () => {
  it.each(bundles)("%s has the same key set as en", (_language, locale) => {
    expect([...flatten(locale).keys()].sort()).toEqual(
      [...english.keys()].sort(),
    );
  });

  it.each(bundles)(
    "%s keeps the same placeholders and inline markup as en",
    (language, locale) => {
      const translated = flatten(locale);
      const mismatches: string[] = [];
      for (const [key, source] of english) {
        const value = translated.get(key);
        if (value === undefined) {
          mismatches.push(`${key}: missing`);
          continue;
        }
        if (value.trim() === "") {
          mismatches.push(`${key}: empty`);
        }
        const sourcePlaceholders = placeholders(source).join("\0");
        const translatedPlaceholders = placeholders(value).join("\0");
        if (sourcePlaceholders !== translatedPlaceholders) {
          mismatches.push(
            `${key}: placeholders ${JSON.stringify(placeholders(source))} vs ${JSON.stringify(placeholders(value))}`,
          );
        }
        const sourceMarkup = inlineMarkup(source).join("\0");
        const translatedMarkup = inlineMarkup(value).join("\0");
        if (sourceMarkup !== translatedMarkup) {
          mismatches.push(
            `${key}: markup ${JSON.stringify(inlineMarkup(source))} vs ${JSON.stringify(inlineMarkup(value))}`,
          );
        }
      }
      expect(mismatches, language).toEqual([]);
    },
  );
});
