import { describe, expect, it, vi } from "vitest";

const pluginCheckMock = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => pluginCheckMock(...args),
}));

import {
  checkForUpdate,
  MANAGED_UPDATES_UNAVAILABLE_CODE,
} from "@/lib/updater";

describe("checkForUpdate", () => {
  it("returns the stable managed-unavailable result without loading the updater plugin", async () => {
    await expect(checkForUpdate({ timeout: 1 })).resolves.toEqual({
      status: "unavailable",
      code: MANAGED_UPDATES_UNAVAILABLE_CODE,
    });
    expect(MANAGED_UPDATES_UNAVAILABLE_CODE).toBe(
      "managed_updates_unavailable",
    );
    expect(pluginCheckMock).not.toHaveBeenCalled();
  });
});
