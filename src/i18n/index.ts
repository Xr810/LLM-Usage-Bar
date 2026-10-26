import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { listen } from "@tauri-apps/api/event";

import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";
import {
  LEGACY_LOCAL_STORAGE_KEYS,
  LOCAL_STORAGE_KEYS,
  readMigratedLocalStorage,
} from "@/lib/localStorageMigration";
import { isTauriRuntime } from "@/lib/platform";
import {
  parseAppLanguage,
  readBackendLanguage,
  resolveInitialLanguage,
  type AppLanguage,
} from "./languageSource";

export interface SettingsChangedPayload {
  keys?: readonly string[];
}

function readStoredLanguage(): AppLanguage | null {
  if (typeof window === "undefined") return null;
  try {
    return readMigratedLocalStorage({
      currentKey: LOCAL_STORAGE_KEYS.language,
      legacyKeys: LEGACY_LOCAL_STORAGE_KEYS.language,
      defaultValue: null,
      parse: parseAppLanguage,
    });
  } catch (error) {
    console.warn("[i18n] Failed to read stored language preference", error);
    return null;
  }
}

function navigatorLanguage(): string | undefined {
  if (typeof navigator === "undefined") return undefined;
  return navigator.language ?? navigator.languages?.[0];
}

/** Backend mock, then a one-time localStorage migration, then the browser language. */
export function resolveCurrentLanguage(): AppLanguage {
  return resolveInitialLanguage({
    backend: readBackendLanguage(),
    stored: readStoredLanguage(),
    navigatorLang: navigatorLanguage(),
  });
}

const resources = {
  en: {
    translation: en,
  },
  ja: {
    translation: ja,
  },
  zh: {
    translation: zh,
  },
  "zh-TW": {
    translation: zhTW,
  },
};

i18n.use(initReactI18next).init({
  resources,
  lng: resolveCurrentLanguage(),
  fallbackLng: "en",

  interpolation: {
    escapeValue: false,
  },

  debug: false,
});

/**
 * C18 `settings-changed`. A payload of `{ keys: ["language"] }` re-reads the
 * language mock. This does not call `get_language`.
 */
export function applyLanguageSettingsChange(
  payload: SettingsChangedPayload | null | undefined,
): Promise<void> {
  if (!payload?.keys?.includes("language")) return Promise.resolve();
  return Promise.resolve(i18n.changeLanguage(resolveCurrentLanguage())).then(
    () => undefined,
  );
}

let languageListenerStarted = false;

/** Subscribes once inside the Tauri runtime. Tests can call this after mocking. */
export function startLanguageSettingsListener(): void {
  if (languageListenerStarted || !isTauriRuntime()) return;
  languageListenerStarted = true;
  void listen<SettingsChangedPayload>("settings-changed", (event) => {
    void applyLanguageSettingsChange(event.payload);
  }).catch((error) => {
    languageListenerStarted = false;
    console.warn("[i18n] Failed to listen for settings-changed", error);
  });
}

startLanguageSettingsListener();

export default i18n;
