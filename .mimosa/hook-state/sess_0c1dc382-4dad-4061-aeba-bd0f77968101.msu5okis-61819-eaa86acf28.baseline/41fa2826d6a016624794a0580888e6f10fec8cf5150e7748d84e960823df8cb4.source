import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  readMigratedLocalStorage,
  type LocalStorageMigration,
} from "@/lib/localStorageMigration";

const migration: LocalStorageMigration<"light" | "dark"> = {
  currentKey: "llm-usage-bar:theme",
  legacyKeys: ["cc-switch-theme"],
  defaultValue: "light",
  parse: (value) => (value === "light" || value === "dark" ? value : null),
};

describe("readMigratedLocalStorage", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("prefers a valid current key without consuming the legacy key", () => {
    window.localStorage.setItem(migration.currentKey, "dark");
    window.localStorage.setItem(migration.legacyKeys[0], "light");

    expect(readMigratedLocalStorage(migration)).toBe("dark");
    expect(window.localStorage.getItem(migration.legacyKeys[0])).toBe("light");
  });

  it("writes a valid legacy value to the current key before removing it", () => {
    window.localStorage.setItem(migration.legacyKeys[0], "dark");

    expect(readMigratedLocalStorage(migration)).toBe("dark");
    expect(window.localStorage.getItem(migration.currentKey)).toBe("dark");
    expect(window.localStorage.getItem(migration.legacyKeys[0])).toBeNull();
  });

  it("tries the legacy key after an invalid current value", () => {
    window.localStorage.setItem(migration.currentKey, "sepia");
    window.localStorage.setItem(migration.legacyKeys[0], "dark");

    expect(readMigratedLocalStorage(migration)).toBe("dark");
    expect(window.localStorage.getItem(migration.currentKey)).toBe("dark");
    expect(window.localStorage.getItem(migration.legacyKeys[0])).toBeNull();
  });

  it("uses the default and keeps an invalid legacy value for diagnosis", () => {
    window.localStorage.setItem(migration.legacyKeys[0], "sepia");

    expect(readMigratedLocalStorage(migration)).toBe("light");
    expect(window.localStorage.getItem(migration.currentKey)).toBeNull();
    expect(window.localStorage.getItem(migration.legacyKeys[0])).toBe("sepia");
  });

  it("returns a valid legacy value even when the best-effort rewrite fails", () => {
    window.localStorage.setItem(migration.legacyKeys[0], "dark");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });

    expect(readMigratedLocalStorage(migration)).toBe("dark");
  });
});
