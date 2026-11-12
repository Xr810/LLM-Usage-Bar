import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { TrayUsageSnapshot } from "./trayUsage";

describe("native bridge tray contract", () => {
  it("keeps unavailable quota distinct from zero", () => {
    const fixture = JSON.parse(
      readFileSync(
        resolve(
          process.cwd(),
          "native/Tests/UsageCoreTests/Fixtures/tray-usage-snapshot-v1.json",
        ),
        "utf8",
      ),
    ) as TrayUsageSnapshot;

    expect(fixture.agents[0].providers).toHaveLength(2);
    const unavailable = fixture.agents[0].providers[0].subscription?.windows[1];
    expect(unavailable?.status).toBe("unknown");
    expect(unavailable?.remainingPercent).toBeNull();
  });
});
