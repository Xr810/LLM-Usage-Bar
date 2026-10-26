import { describe, expect, it } from "vitest";
import type { LubBuild, LubTier } from "./panelModel";
import { proTagVisible, upgradeAffordanceVisible } from "./upgradeEntry";

const builds: LubBuild[] = ["community", "official"];
const tiers: LubTier[] = ["free", "pro", "pro_p3"];

describe("upgrade entry gating", () => {
  it("shows the PRO tag and panel only for official Free with upsellEntry", () => {
    const cases: {
      build: LubBuild;
      tier: LubTier;
      upsellEntry: boolean;
      affordance: boolean;
      proTag: boolean;
    }[] = [];
    for (const build of builds) {
      for (const tier of tiers) {
        for (const upsellEntry of [false, true]) {
          const input = { build, tier, upsellEntry };
          const affordance = upgradeAffordanceVisible(input);
          const proTag = proTagVisible(input);
          cases.push({ build, tier, upsellEntry, affordance, proTag });
          expect(proTag).toBe(affordance && tier === "free");
        }
      }
    }
    expect(cases.filter((row) => row.proTag)).toEqual([
      {
        build: "official",
        tier: "free",
        upsellEntry: true,
        affordance: true,
        proTag: true,
      },
    ]);
    expect(
      cases.filter((row) => row.affordance).map((row) => row.tier),
    ).toEqual(["free", "pro", "pro_p3"]);
    expect(
      cases.filter((row) => row.build === "community" && row.affordance),
    ).toEqual([]);
  });

  it("treats loading and failure as community, with no upgrade entry", () => {
    for (const pending of [true, false]) {
      for (const failed of [true, false]) {
        if (!pending && !failed) continue;
        const input = {
          build: "official" as const,
          tier: "free" as const,
          upsellEntry: true,
          pending,
          failed,
        };
        expect(upgradeAffordanceVisible(input)).toBe(false);
        expect(proTagVisible(input)).toBe(false);
      }
    }
    expect(
      upgradeAffordanceVisible({
        build: "official",
        tier: "free",
        upsellEntry: true,
      }),
    ).toBe(true);
    expect(
      proTagVisible({ build: null, tier: "free", upsellEntry: true }),
    ).toBe(false);
    expect(proTagVisible({ tier: "free", upsellEntry: true })).toBe(false);
    expect(
      upgradeAffordanceVisible({
        build: "official",
        tier: "free",
        upsellEntry: undefined,
      }),
    ).toBe(false);
  });
});
