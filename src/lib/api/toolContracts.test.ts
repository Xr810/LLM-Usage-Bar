import { describe, expect, it, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import fixture from "../../../tests/fixtures/cli-contract.json";
import { settingsApi } from "./settings";
import {
  toolVersionSchema,
  toolInstallationReportSchema,
  wslShellPreferenceSchema,
} from "./toolContracts";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("CLI wire contract (shared with Rust serialization)", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("preserves response names, nulls, and private-field exclusion", () => {
    expect(toolVersionSchema.parse(fixture.version)).toEqual(fixture.version);
    expect(toolInstallationReportSchema.parse(fixture.report)).toEqual(
      fixture.report,
    );
    expect(wslShellPreferenceSchema.parse(fixture.wsl)).toEqual(fixture.wsl);
    expect(wslShellPreferenceSchema.parse({})).toEqual({});
    expect(
      wslShellPreferenceSchema.parse({ wslShell: null, wslShellFlag: null }),
    ).toEqual({ wslShell: null, wslShellFlag: null });
    expect(
      toolVersionSchema.safeParse({
        ...fixture.version,
        installed_but_broken: undefined,
      }).success,
    ).toBe(false);
    expect(
      toolInstallationReportSchema.safeParse({
        ...fixture.report,
        installs: [{ ...fixture.report.installs[0], real: "/internal" }],
      }).success,
    ).toBe(false);
  });

  it("sends camelCase command arguments without renaming snake_case responses", async () => {
    const wslShellByTool = { codex: fixture.wsl };
    vi.mocked(invoke).mockResolvedValueOnce([fixture.version]);
    expect(
      await settingsApi.getToolVersions(["codex"], wslShellByTool),
    ).toEqual([fixture.version]);
    expect(invoke).toHaveBeenLastCalledWith("get_tool_versions", {
      tools: ["codex"],
      wslShellByTool,
    });
    vi.mocked(invoke).mockResolvedValueOnce([fixture.report]);
    expect(await settingsApi.probeToolInstallations(["codex"])).toEqual([
      fixture.report,
    ]);
    expect(invoke).toHaveBeenLastCalledWith("probe_tool_installations", {
      tools: ["codex"],
    });
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    await settingsApi.runToolLifecycleAction(
      ["codex"],
      "update",
      wslShellByTool,
    );
    expect(invoke).toHaveBeenLastCalledWith("run_tool_lifecycle_action", {
      tools: ["codex"],
      action: "update",
      wslShellByTool,
    });
  });

  it("propagates backend failure rather than returning an empty success", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("No supported tools selected");
    await expect(settingsApi.probeToolInstallations([])).rejects.toBe(
      "No supported tools selected",
    );
  });
});
