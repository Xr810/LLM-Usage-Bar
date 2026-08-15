export const LOCAL_STORAGE_KEYS = {
  language: "llm-usage-bar:language",
  theme: "llm-usage-bar:theme",
  lastApp: "llm-usage-bar:last-app",
  dismissedUpdateVersion: "llm-usage-bar:update:dismissed-version",
  sessionListViewMode: "llm-usage-bar:session-manager:list-view-mode",
  sessionGroupExpansion: "llm-usage-bar:session-manager:group-expansion",
} as const;

export const LEGACY_LOCAL_STORAGE_KEYS = {
  language: ["language"],
  theme: ["cc-switch-theme"],
  lastApp: ["cc-switch-last-app"],
  dismissedUpdateVersion: [
    "ccswitch:update:dismissedVersion",
    "dismissedUpdateVersion",
  ],
  sessionListViewMode: ["cc-switch.sessionManager.listViewMode"],
  sessionGroupExpansion: ["cc-switch.sessionManager.groupExpansionState"],
} as const;

export interface LocalStorageMigration<T> {
  currentKey: string;
  legacyKeys: readonly string[];
  defaultValue: T;
  parse: (value: string) => T | null;
}

const getLocalStorage = (): Storage | null => {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
};

export function readMigratedLocalStorage<T>({
  currentKey,
  legacyKeys,
  defaultValue,
  parse,
}: LocalStorageMigration<T>): T {
  const storage = getLocalStorage();
  if (!storage) return defaultValue;

  try {
    const currentValue = storage.getItem(currentKey);
    if (currentValue !== null) {
      const parsed = parse(currentValue);
      if (parsed !== null) return parsed;
    }

    for (const legacyKey of legacyKeys) {
      const legacyValue = storage.getItem(legacyKey);
      if (legacyValue === null) continue;

      const parsed = parse(legacyValue);
      if (parsed === null) continue;

      try {
        storage.setItem(currentKey, legacyValue);
        storage.removeItem(legacyKey);
      } catch {
        // The valid preference still wins even if the best-effort rewrite fails.
      }
      return parsed;
    }
  } catch {
    return defaultValue;
  }

  return defaultValue;
}

export function writeMigratedLocalStorage(
  currentKey: string,
  legacyKeys: readonly string[],
  value: string,
): void {
  const storage = getLocalStorage();
  if (!storage) return;

  try {
    storage.setItem(currentKey, value);
    for (const legacyKey of legacyKeys) {
      storage.removeItem(legacyKey);
    }
  } catch {
    // Preferences are best-effort when localStorage is unavailable.
  }
}

export function removeMigratedLocalStorage(
  currentKey: string,
  legacyKeys: readonly string[],
): void {
  const storage = getLocalStorage();
  if (!storage) return;

  try {
    storage.removeItem(currentKey);
    for (const legacyKey of legacyKeys) {
      storage.removeItem(legacyKey);
    }
  } catch {
    // Preferences are best-effort when localStorage is unavailable.
  }
}
