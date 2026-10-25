import { afterEach, describe, expect, it } from "vitest";
import {
  LEGACY_LOCAL_STORAGE_KEYS,
  LOCAL_STORAGE_KEYS,
  readMigratedLocalStorage,
} from "@/lib/localStorageMigration";
import {
  parseAppLanguage,
  resolveInitialLanguage,
  setBackendLanguage,
} from "./languageSource";

const languageRead = () =>
  readMigratedLocalStorage({
    currentKey: LOCAL_STORAGE_KEYS.language,
    legacyKeys: LEGACY_LOCAL_STORAGE_KEYS.language,
    defaultValue: null,
    parse: parseAppLanguage,
  });

afterEach(() => {
  localStorage.clear();
  setBackendLanguage(null);
});

describe("language source", () => {
  it("migrates a legacy language once", () => {
    localStorage.setItem("language", "en");
    expect(languageRead()).toBe("en");
    expect(localStorage.getItem(LOCAL_STORAGE_KEYS.language)).toBe("en");
    expect(localStorage.getItem("language")).toBeNull();

    localStorage.setItem("language", "ja");
    expect(languageRead()).toBe("en");
    expect(localStorage.getItem("language")).toBe("ja");
  });

  it("prefers the backend language over a stored value and the navigator", () => {
    expect(
      resolveInitialLanguage({
        backend: "en",
        stored: "zh",
        navigatorLang: "ja",
      }),
    ).toBe("en");
    expect(
      resolveInitialLanguage({
        backend: null,
        stored: "zh-TW",
        navigatorLang: "en-US",
      }),
    ).toBe("zh-TW");
    expect(
      resolveInitialLanguage({
        backend: null,
        stored: null,
        navigatorLang: "zh-HK",
      }),
    ).toBe("zh-TW");
    expect(
      resolveInitialLanguage({
        backend: null,
        stored: null,
        navigatorLang: "fr",
      }),
    ).toBe("zh");
  });
});
