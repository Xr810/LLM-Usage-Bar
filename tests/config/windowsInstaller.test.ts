import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const config = JSON.parse(
  readFileSync("src-tauri/tauri.windows.conf.json", "utf8"),
);
// Follow the active config, not the unused per-user template.
const template = readFileSync(
  join("src-tauri", config.bundle.windows?.wix?.template ?? "missing-template"),
  "utf8",
);
const xml = new DOMParser().parseFromString(template, "application/xml");
const setters = Array.from(
  xml.querySelectorAll('SetProperty[Id="INSTALLDIR"]'),
);

// Evaluate only the conjunction/property-presence grammar used by these Type 51
// conditions. This checks the authored policy, NOT Windows Installer execution.
function applyDefaults(properties: Record<string, string>): string | undefined {
  for (const setter of setters) {
    const enabled = setter
      .textContent!.trim()
      .split(/\s+AND\s+/)
      .every((term) => {
        const match = /^(NOT )?([A-Z]+)$/.exec(term);
        if (!match) throw new Error(`Unsupported MSI condition: ${term}`);
        return match[1] ? !properties[match[2]] : !!properties[match[2]];
      });
    if (enabled) {
      const value = /^\[([A-Z]+)\]$/.exec(setter.getAttribute("Value")!)!;
      properties.INSTALLDIR = properties[value[1]];
    }
  }
  return properties.INSTALLDIR;
}

describe("active Windows MSI install-directory policy", () => {
  it("wires the per-machine template and isolates AppSearch from INSTALLDIR", () => {
    expect(config.bundle.windows.wix.template).toBe("wix/main.wxs");
    expect(config.bundle.targets).toEqual(["nsis", "msi"]);
    expect(xml.querySelector("parsererror")).toBeNull();
    expect(xml.querySelector("Package")?.getAttribute("InstallScope")).toBe(
      "perMachine",
    );
    expect(
      xml.querySelectorAll('Property[Id="INSTALLDIR"] RegistrySearch'),
    ).toHaveLength(0);
    for (const id of [
      "INSTALLDIR",
      "PREVIOUSMSIINSTALLDIR",
      "PREVIOUSNSISINSTALLDIR",
    ]) {
      expect(
        xml.querySelector(`Property[Id="${id}"]`)?.getAttribute("Secure"),
      ).toBe("yes");
    }
    const msi = xml.querySelector(
      'Property[Id="PREVIOUSMSIINSTALLDIR"] RegistrySearch',
    )!;
    const nsis = xml.querySelector(
      'Property[Id="PREVIOUSNSISINSTALLDIR"] RegistrySearch',
    )!;
    expect(msi.getAttribute("Name")).toBe("InstallDir");
    expect(nsis.hasAttribute("Name")).toBe(false);
    for (const search of [msi, nsis]) {
      expect(search.getAttribute("Root")).toBe("HKCU");
      expect(search.getAttribute("Key")).toBe(
        "Software\\\\{{manufacturer}}\\\\{{product_name}}",
      );
      expect(search.getAttribute("Type")).toBe("raw");
    }
    expect(setters).toHaveLength(2);
    expect(setters[0].getAttribute("After")).toBe("AppSearch");
    expect(setters[1].getAttribute("After")).toBe(
      setters[0].getAttribute("Action"),
    );
    expect(new Set(setters.map((s) => s.getAttribute("Action"))).size).toBe(2);
    for (const setter of setters)
      expect(setter.getAttribute("Sequence")).toBe("both");
  });

  it.each([
    ["no previous values", "", "", undefined],
    ["NSIS only", "C:\\old-nsis", "", "C:\\old-nsis"],
    ["MSI only", "", "D:\\old-msi", "D:\\old-msi"],
    ["dual values prefer MSI", "C:\\old-nsis", "D:\\old-msi", "D:\\old-msi"],
  ])(
    "preserves explicit paths and historical defaults: %s",
    (_name, nsis, msi, expected) => {
      const history = {
        PREVIOUSNSISINSTALLDIR: nsis!,
        PREVIOUSMSIINSTALLDIR: msi!,
      };
      expect(applyDefaults({ ...history })).toBe(expected);
      expect(
        applyDefaults({ ...history, INSTALLDIR: "E:\\requested path" }),
      ).toBe("E:\\requested path");
      // Full UI runs defaults once, then the execute sequence must preserve the
      // user's changed directory, including when AppSearch is skipped there.
      const ui = { ...history, INSTALLDIR: "" };
      applyDefaults(ui);
      ui.INSTALLDIR = "F:\\chosen in UI";
      expect(applyDefaults(ui)).toBe("F:\\chosen in UI");
    },
  );

  it.each(["Installed", "WIX_UPGRADE_DETECTED"])(
    "does not disable historical-directory recovery during %s",
    (mode) => {
      expect(
        applyDefaults({
          [mode]: "1",
          PREVIOUSNSISINSTALLDIR: "C:\\stale-nsis",
          PREVIOUSMSIINSTALLDIR: "D:\\existing-msi",
        }),
      ).toBe("D:\\existing-msi");
    },
  );

  it.each([
    ["LF", "\n"],
    ["CRLF", "\r\n"],
  ])(
    "keeps upstream default, components, upgrade and repair authoring unchanged with %s checkout",
    (_name, lineEnding) => {
      // Independently calculated from tauri-cli-v2.11.4's published main.wxs,
      // excluding ONLY the install-directory search/default block. No network
      // dependency in tests. Any further upstream update requires explicit review.
      const checkout = template
        .replace(/\r\n/g, "\n")
        .replace(/\n/g, lineEnding);
      const unchanged = checkout
        .replace(
          /        <!-- Based on[\s\S]*?(?=        <!-- launch app checkbox -->)/,
          "",
        )
        // Git text=auto permits native CRLF checkouts on Windows. Only line
        // endings are canonicalized; every other upstream byte stays pinned.
        .replace(/\r\n/g, "\n");
      expect(createHash("sha256").update(unchanged).digest("hex")).toBe(
        "545199fed438ccb7b3733b84592653a8db16131ea9626c8f364f96b2530e415a",
      );
    },
  );
});
