import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parse as parseToml } from "smol-toml";
import { settingsSchema } from "../../src/lib/schemas/settings";
import {
  classifyOldIdentityOccurrence,
  oldIdentityOccurrences,
} from "./productIdentityAllowlist";

describe("product identity manifests", () => {
  it("uses the LLM Usage Bar package and bundle identity", () => {
    const pkg = JSON.parse(readFileSync("package.json", "utf8"));
    const cargo = parseToml(readFileSync("src-tauri/Cargo.toml", "utf8")) as {
      package: { name: string };
      lib: { name: string };
    };
    const tauri = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));

    expect(pkg.name).toBe("llm-usage-bar");
    expect(cargo.package.name).toBe("llm-usage-bar");
    expect(cargo.lib.name).toBe("llm_usage_bar_lib");
    expect(tauri.productName).toBe("LLM Usage Bar");
    expect(tauri.identifier).toBe("com.llmusagebar.desktop");
  });
});

describe("app-owned identity discriminators", () => {
  it("accepts only the current writable skill storage values", () => {
    const base = {
      showInTray: true,
      minimizeToTrayOnClose: true,
    };

    expect(
      settingsSchema.safeParse({
        ...base,
        skillStorageLocation: "llm_usage_bar",
      }).success,
    ).toBe(true);
    expect(
      settingsSchema.safeParse({
        ...base,
        skillStorageLocation: "unified",
      }).success,
    ).toBe(true);
    expect(
      settingsSchema.safeParse({
        ...base,
        skillStorageLocation: "cc_switch",
      }).success,
    ).toBe(false);
  });

  it("uses exact file and source context for old identity classification", () => {
    expect(oldIdentityOccurrences.length).toBeGreaterThan(0);
    expect(
      classifyOldIdentityOccurrence(
        "src-tauri/src/settings.rs",
        99,
        '    "cc-switch-sync".to_string()',
      )?.kind,
    ).toBe("externalWireStable");
    expect(
      classifyOldIdentityOccurrence(
        "src-tauri/src/product_identity.rs",
        11,
        'pub const LEGACY_DATA_DIR: &str = ".cc-switch";',
      )?.kind,
    ).toBe("legacyReadOnly");
    expect(
      classifyOldIdentityOccurrence(
        "src/unknown.ts",
        1,
        'const product = "cc-switch";',
      ),
    ).toBeUndefined();
  });

  it("removes owned Task 4 literals and pins approved compatibility bytes", () => {
    for (const occurrence of oldIdentityOccurrences) {
      const source = readFileSync(occurrence.file, "utf8");
      if (occurrence.kind === "ownedRename") {
        expect(
          source,
          `${occurrence.file} still contains ${occurrence.context}`,
        ).not.toContain(occurrence.context);
      } else {
        const actualLine = source
          .split("\n")
          [occurrence.lineNumber - 1]?.trim();
        expect(
          actualLine,
          `${occurrence.file}:${occurrence.lineNumber} changed`,
        ).toBe(occurrence.context);
      }
    }
  });
});
