import { afterEach, describe, expect, it, vi } from "vitest";
import {
  LEGACY_LOCAL_STORAGE_KEYS,
  LOCAL_STORAGE_KEYS,
  readMigratedLocalStorage,
} from "@/lib/localStorageMigration";
import i18n, {
  applyLanguageSettingsChange,
  startLanguageSettingsListener,
} from "./index";
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

afterEach(async () => {
  localStorage.clear();
  setBackendLanguage(null);
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  await i18n.changeLanguage("zh");
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
    ).toBe("en");
    expect(
      resolveInitialLanguage({
        backend: null,
        stored: null,
      }),
    ).toBe("en");
    expect(
      resolveInitialLanguage({
        backend: null,
        stored: null,
        navigatorLang: "de-DE",
      }),
    ).toBe("en");
  });

  it("re-reads language when settings-changed lists language", async () => {
    setBackendLanguage("en");
    await applyLanguageSettingsChange({ keys: ["language"] });
    expect(i18n.language).toBe("en");

    setBackendLanguage("ja");
    await applyLanguageSettingsChange({ keys: ["theme"] });
    expect(i18n.language).toBe("en");
  });

  it("listens for the tauri settings-changed event", async () => {
    (window as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    startLanguageSettingsListener();
    setBackendLanguage("ja");
    const { emitTauriEvent } = await import("../../tests/msw/tauriMocks");
    emitTauriEvent("settings-changed", { keys: ["language"] });
    await vi.waitFor(() => {
      expect(i18n.language).toBe("ja");
    });
  });
});
