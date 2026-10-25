import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import zhTW from "./locales/zh-TW.json";
import {
  LEGACY_LOCAL_STORAGE_KEYS,
  LOCAL_STORAGE_KEYS,
  readMigratedLocalStorage,
} from "@/lib/localStorageMigration";
import {
  parseAppLanguage,
  readBackendLanguage,
  resolveInitialLanguage,
} from "./languageSource";

const getInitialLanguage = () => {
  let stored = null;
  if (typeof window !== "undefined") {
    try {
      stored = readMigratedLocalStorage({
        currentKey: LOCAL_STORAGE_KEYS.language,
        legacyKeys: LEGACY_LOCAL_STORAGE_KEYS.language,
        defaultValue: null,
        parse: parseAppLanguage,
      });
    } catch (error) {
      console.warn("[i18n] Failed to read stored language preference", error);
    }
  }

  const navigatorLang =
    typeof navigator !== "undefined"
      ? (navigator.language ?? navigator.languages?.[0])
      : undefined;

  return resolveInitialLanguage({
    backend: readBackendLanguage(),
    stored,
    navigatorLang,
  });
};

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
  lng: getInitialLanguage(), // 根据本地存储或系统语言选择默认语言
  fallbackLng: "en", // 如果缺少中文翻译则退回英文

  interpolation: {
    escapeValue: false, // React 已经默认转义
  },

  // 开发模式下显示调试信息
  debug: false,
});

export default i18n;
