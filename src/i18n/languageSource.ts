export type AppLanguage = "zh" | "zh-TW" | "en" | "ja";

/**
 * Stand-in for C8 `get_language()`. The real command is not called.
 * A null value means the backend has no language yet.
 */
let backendLanguage: AppLanguage | null = null;

export function readBackendLanguage(): AppLanguage | null {
  return backendLanguage;
}

export function setBackendLanguage(language: AppLanguage | null): void {
  backendLanguage = language;
}

export function parseAppLanguage(
  value: string | null | undefined,
): AppLanguage | null {
  return value === "zh" || value === "zh-TW" || value === "en" || value === "ja"
    ? value
    : null;
}

export function resolveInitialLanguage(input: {
  backend: AppLanguage | null;
  stored: AppLanguage | null;
  navigatorLang?: string;
}): AppLanguage {
  if (input.backend) return input.backend;
  if (input.stored) return input.stored;

  const navigatorLang = input.navigatorLang?.toLowerCase();
  if (navigatorLang === "zh") return "zh";
  if (
    navigatorLang?.startsWith("zh-tw") ||
    navigatorLang?.startsWith("zh-hk") ||
    navigatorLang?.startsWith("zh-mo") ||
    navigatorLang?.startsWith("zh-hant")
  ) {
    return "zh-TW";
  }
  if (navigatorLang?.startsWith("zh")) return "zh";
  if (navigatorLang?.startsWith("ja")) return "ja";
  if (navigatorLang?.startsWith("en")) return "en";
  // Unmapped tags, and a missing navigator language, fall back to English.
  return "en";
}
