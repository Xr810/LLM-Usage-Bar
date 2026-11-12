import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { NativeProviderDashboardV1 } from "./nativeBridge";
import type {
  AgentUsageBreakdownView,
  ModelUsageDashboardView,
  UsageEventPage,
} from "./usageDashboard";

interface Envelope<T> {
  schemaVersion: number;
  data: T;
}

interface DashboardFixture {
  providerDashboard: Envelope<NativeProviderDashboardV1>;
  modelDashboard: Envelope<ModelUsageDashboardView>;
  agentBreakdown: Envelope<AgentUsageBreakdownView>;
  usageEvents: Envelope<UsageEventPage>;
}

describe("native bridge dashboard contract", () => {
  it("preserves account identity and unavailable values", () => {
    const fixture = JSON.parse(
      readFileSync(
        resolve(
          process.cwd(),
          "native/Tests/UsageCoreTests/Fixtures/dashboard-contract-v1.json",
        ),
        "utf8",
      ),
    ) as DashboardFixture;

    const provider = fixture.providerDashboard.data.providers[0];
    expect(fixture.providerDashboard.schemaVersion).toBe(1);
    expect(provider.provider.id).toBe("codex-work");
    expect(Object.keys(provider.provider).sort()).toEqual([
      "billingKind",
      "enabled",
      "id",
      "name",
      "productGroupId",
      "systemPresetKey",
    ]);
    expect(provider.provider).not.toHaveProperty("routeBaseUrl");
    expect(provider.provider).not.toHaveProperty("apiKeys");
    expect(provider.provider).not.toHaveProperty("bindings");
    expect(provider.totalCostUsd).toBeNull();
    expect(provider.quota?.sevenDayUtilizationPercent).toBeNull();
    expect(fixture.modelDashboard.data.totalTokens).toBe(2_600);
    expect(fixture.agentBreakdown.data.agents[0].agentModuleId).toBe("codex");
    expect(fixture.usageEvents.data.items[0].totalCostUsd).toBeNull();
  });
});
