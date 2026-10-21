import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

interface NativeSettingsV1 {
  launchOnStartup: boolean;
  silentStartup: boolean;
  showInTray: boolean;
  minimizeToTrayOnClose: boolean;
  language: "en" | "zh" | "zh-TW" | "ja" | null;
  usageWarningRemainingPercent: number;
  usageCriticalRemainingPercent: number;
  apiBudgetMode: "shared" | "perProvider";
  sharedApiDailyBudgetUsd: string | null;
  usageDashboardRefreshIntervalMs: number | null;
}

interface NativeSettingsDocumentV1 {
  schemaVersion: number;
  data: NativeSettingsV1;
  revision: string;
}

describe("native bridge settings contract", () => {
  it("matches the explicit safe projection shape", () => {
    const fixture = JSON.parse(
      readFileSync(
        resolve(
          process.cwd(),
          "native/Tests/UsageCoreTests/Fixtures/native-settings-v1.json",
        ),
        "utf8",
      ),
    ) as NativeSettingsDocumentV1;

    expect(fixture.schemaVersion).toBe(1);
    expect(fixture.revision).toMatch(/^[0-9a-f]{32}$/);
    expect(Object.keys(fixture.data).sort()).toEqual([
      "apiBudgetMode",
      "language",
      "launchOnStartup",
      "minimizeToTrayOnClose",
      "sharedApiDailyBudgetUsd",
      "showInTray",
      "silentStartup",
      "usageCriticalRemainingPercent",
      "usageDashboardRefreshIntervalMs",
      "usageWarningRemainingPercent",
    ]);
    expect(fixture.data.apiBudgetMode).toBe("perProvider");
    expect(fixture.data.sharedApiDailyBudgetUsd).toBe("12.50");
  });
});
