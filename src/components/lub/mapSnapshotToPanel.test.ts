import { describe, expect, it } from "vitest";
import type { SourceRowView, TrayUsageSnapshot } from "@/types/trayUsage";
import { trayUsageSnapshotFixture } from "../../../tests/msw/handlers";
import { mapSnapshotToPanel } from "./mapSnapshotToPanel";

function cloneSnapshot(): TrayUsageSnapshot {
  return structuredClone(trayUsageSnapshotFixture);
}

function sourceRow(
  providerId: string,
  overrides: Partial<SourceRowView> = {},
): SourceRowView {
  return {
    providerId,
    displayName: providerId,
    billingKind: "subscription",
    enforceable: false,
    progress: 0,
    stopped: false,
    sourceState: "ok",
    noteCodes: [],
    nextResetAt: null,
    ...overrides,
  };
}

describe("mapSnapshotToPanel BE-21b fields", () => {
  it("copies buildType and leaves notes and resets empty when sources omit them", () => {
    const model = mapSnapshotToPanel(cloneSnapshot(), () => undefined);
    expect(model.buildType).toBe("community");
    expect(model.sources.length).toBeGreaterThan(0);
    for (const source of model.sources) {
      expect(source.noteCodes).toBeNull();
      expect(source.nextResetAt).toBeNull();
    }
  });

  it("copies row noteCodes and nextResetAt and does not reuse window resetsAt", () => {
    const snapshot = cloneSnapshot();
    snapshot.buildType = "official";
    const provider = snapshot.agents[0]?.providers[0];
    expect(provider).toBeDefined();
    if (!provider) return;
    const windowReset = provider.subscription?.windows[0]?.resetsAt;
    expect(windowReset).toBeTruthy();
    snapshot.sources = [
      sourceRow(provider.providerId, {
        noteCodes: ["codex_websocket"],
        nextResetAt: "2026-10-27T16:30:00.000Z",
      }),
    ];
    const other = snapshot.agents[0]?.providers[1];
    if (other) {
      snapshot.sources.push(
        sourceRow(other.providerId, { noteCodes: [], nextResetAt: null }),
      );
    }

    const model = mapSnapshotToPanel(snapshot, () => undefined);
    expect(model.buildType).toBe("official");
    const copied = model.sources.find(
      (source) => source.id === provider.providerId,
    );
    expect(copied?.noteCodes).toEqual(["codex_websocket"]);
    expect(copied?.nextResetAt).toBe("2026-10-27T16:30:00.000Z");
    expect(copied?.nextResetAt).not.toBe(windowReset);
    if (other) {
      const blank = model.sources.find(
        (source) => source.id === other.providerId,
      );
      expect(blank?.noteCodes).toBeNull();
      expect(blank?.nextResetAt).toBeNull();
    }
  });

  it("hides a null or unparseable nextResetAt and an empty note list", () => {
    const snapshot = cloneSnapshot();
    const provider = snapshot.agents[0]?.providers[0];
    expect(provider).toBeDefined();
    if (!provider) return;
    snapshot.sources = [
      sourceRow(provider.providerId, {
        noteCodes: [],
        nextResetAt: "tomorrow",
      }),
    ];

    const model = mapSnapshotToPanel(snapshot, () => undefined);
    const row = model.sources.find(
      (source) => source.id === provider.providerId,
    );
    expect(row?.noteCodes).toBeNull();
    expect(row?.nextResetAt).toBeNull();
  });

  it("alerts only when the budget level is reached", () => {
    const reached = cloneSnapshot();
    reached.apiBudget.level = "reached";
    reached.apiBudget.status = "green";
    reached.apiBudget.budgetConsumedPercent = "10";
    expect(mapSnapshotToPanel(reached, () => undefined).state).toBe("over");

    const deprecated = cloneSnapshot();
    deprecated.apiBudget.level = "warn";
    deprecated.apiBudget.status = "red";
    deprecated.apiBudget.budgetConsumedPercent = "100";
    expect(mapSnapshotToPanel(deprecated, () => undefined).state).toBe("ok");

    const quiet = cloneSnapshot();
    quiet.apiBudget.level = "ok";
    quiet.apiBudget.status = "red";
    quiet.apiBudget.budgetConsumedPercent = "120";
    expect(mapSnapshotToPanel(quiet, () => undefined).state).toBe("ok");
  });
});
