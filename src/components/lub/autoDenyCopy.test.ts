import { describe, expect, it } from "vitest";
import i18n from "i18next";
import { ensureProCopy } from "@/i18n/proCopy";
import {
  AUTO_DENY_SECONDS_DEFAULT,
  clampAutoDenySeconds,
  hardstopAllowEachNote,
  identityAutoDenyNote,
} from "./autoDenyCopy";

describe("auto-deny seconds", () => {
  it("clamps to 15–240 and defaults to 60", () => {
    expect(AUTO_DENY_SECONDS_DEFAULT).toBe(60);
    expect(clampAutoDenySeconds(undefined)).toBe(60);
    expect(clampAutoDenySeconds(Number.NaN)).toBe(60);
    expect(clampAutoDenySeconds(14)).toBe(15);
    expect(clampAutoDenySeconds(15)).toBe(15);
    expect(clampAutoDenySeconds(60)).toBe(60);
    expect(clampAutoDenySeconds(240)).toBe(240);
    expect(clampAutoDenySeconds(241)).toBe(240);
    expect(clampAutoDenySeconds(60.6)).toBe(61);
  });

  it("passes seconds into the hard-stop and identity notes", async () => {
    ensureProCopy();
    await i18n.changeLanguage("zh");
    const t = i18n.t.bind(i18n);
    const zhHard = hardstopAllowEachNote(t, 15);
    const zhIdentity = identityAutoDenyNote(t, 240);
    expect(zhHard.startsWith("15 秒")).toBe(true);
    expect(zhHard).not.toContain("60 秒");
    expect(zhIdentity.startsWith("240 秒")).toBe(true);
    expect(hardstopAllowEachNote(t)).toContain("60 秒");
    expect(identityAutoDenyNote(t, 9)).toContain("15 秒");
  });
});
