import { describe, expect, it } from "vitest";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { trayUsageSnapshotFixture } from "../../../tests/msw/handlers";
import { mapSnapshotToPanel } from "./mapSnapshotToPanel";

function cloneSnapshot(): TrayUsageSnapshot {
  return structuredClone(trayUsageSnapshotFixture);
}

describe("mapSnapshotToPanel BE-21b fields", () => {
  it("leaves build, notes, and reset empty when the snapshot omits them", () => {
    const model = mapSnapshotToPanel(cloneSnapshot(), () => undefined);
    expect(model.buildType).toBeNull();
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
    provider.noteCodes = ["codex_websocket"];
    provider.nextResetAt = "2026-10-27T16:30:00.000Z";
    const other = snapshot.agents[0]?.providers[1];
    if (other) {
      other.noteCodes = [];
      other.nextResetAt = null;
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

  it("drops an unknown build, blank note codes, and an unparseable reset", () => {
    const snapshot = cloneSnapshot();
    snapshot.buildType = "nightly" as TrayUsageSnapshot["buildType"];
    const provider = snapshot.agents[0]?.providers[0];
    expect(provider).toBeDefined();
    if (!provider) return;
    provider.noteCodes = ["", "  "];
    provider.nextResetAt = "tomorrow";

    const model = mapSnapshotToPanel(snapshot, () => undefined);
    expect(model.buildType).toBeNull();
    const row = model.sources.find(
      (source) => source.id === provider.providerId,
    );
    expect(row?.noteCodes).toBeNull();
    expect(row?.nextResetAt).toBeNull();
  });
});
