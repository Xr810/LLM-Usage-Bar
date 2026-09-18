import { readdirSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { parse as parseToml } from "smol-toml";
import { settingsSchema } from "../../src/lib/schemas/settings";
import {
  classifyOldIdentityOccurrence,
  findOldIdentityMatches,
  oldIdentityOccurrences,
} from "./productIdentityAllowlist";

const currentIdentityTextExtensions = new Set([
  ".css",
  ".html",
  ".json",
  ".md",
  ".plist",
  ".rs",
  ".toml",
  ".ts",
  ".tsx",
  ".yaml",
  ".yml",
]);

function collectCurrentIdentityFiles(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      return collectCurrentIdentityFiles(path);
    }
    return currentIdentityTextExtensions.has(extname(entry.name)) ? [path] : [];
  });
}

function runtimeAndCurrentDocs(): string[] {
  const roots = ["src", "src-tauri/src", "src-tauri/tests", ".github"];
  const exactFiles = [
    "package.json",
    "src-tauri/Cargo.toml",
    "src-tauri/Info.plist",
    "src-tauri/tauri.conf.json",
    "README.md",
    "README_ZH.md",
    "README_ZH_TW.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "SUPPORT.md",
  ];
  return [
    ...new Set([...roots.flatMap(collectCurrentIdentityFiles), ...exactFiles]),
  ].sort();
}

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
        "src-tauri/src/product_identity.rs",
        44,
        "cc-switch",
        'pub const LEGACY_SYNC_REMOTE_ROOT: &str = "cc-switch-sync";',
      )?.kind,
    ).toBe("externalWireStable");
    expect(
      classifyOldIdentityOccurrence(
        "src-tauri/src/product_identity.rs",
        37,
        "cc-switch",
        'pub const LEGACY_DATA_DIR: &str = ".cc-switch";',
      )?.kind,
    ).toBe("legacyReadOnly");
    expect(
      classifyOldIdentityOccurrence(
        "src/unknown.ts",
        18,
        "cc-switch",
        'const product = "cc-switch";',
      ),
    ).toBeUndefined();
    // 同一行搬到文件的哪一行都不影响分类：锚点是内容，不是行号。
    expect(
      classifyOldIdentityOccurrence(
        "src-tauri/src/product_identity.rs",
        37,
        "cc-switch",
        '  pub const LEGACY_DATA_DIR: &str = ".cc-switch";  ',
      )?.kind,
    ).toBe("legacyReadOnly");
  });

  it("removes owned Task 4 literals and pins approved compatibility bytes", () => {
    for (const occurrence of oldIdentityOccurrences) {
      const source = readFileSync(occurrence.file, "utf8");
      if (occurrence.kind === "ownedRename") {
        expect(
          source,
          `${occurrence.file} still contains ${occurrence.context}`,
        ).not.toContain(occurrence.context);
        continue;
      }

      // 按内容数出现次数。行号变了无所谓；次数变了才说明真的多出/少了一处。
      const actualLines = source
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line === occurrence.context);
      expect(
        actualLines.length,
        `${occurrence.file} should contain ${occurrence.occurrences}x "${occurrence.context}" but has ${actualLines.length}`,
      ).toBe(occurrence.occurrences);

      const actualMatch = findOldIdentityMatches(occurrence.context).find(
        (match) => match.columnNumber === occurrence.columnNumber,
      );
      expect(
        actualMatch?.matchedText,
        `${occurrence.file}:${occurrence.columnNumber} "${occurrence.context}" changed`,
      ).toBe(occurrence.matchedText);
    }
  });

  it("classifies every exact old-identity match in macOS runtime and current docs", () => {
    const unclassified: string[] = [];
    const ownedRename: string[] = [];

    for (const file of runtimeAndCurrentDocs()) {
      for (const [lineIndex, sourceLine] of readFileSync(file, "utf8")
        .split("\n")
        .entries()) {
        const line = sourceLine.trim();
        for (const match of findOldIdentityMatches(line)) {
          const occurrence = classifyOldIdentityOccurrence(
            file,
            match.columnNumber,
            match.matchedText,
            line,
          );
          const location = `${file}:${lineIndex + 1}:${match.columnNumber} ${line}`;
          if (!occurrence) {
            unclassified.push(location);
          } else if (occurrence.kind === "ownedRename") {
            ownedRename.push(location);
          }
        }
      }
    }

    expect({ unclassified, ownedRename }).toEqual({
      unclassified: [],
      ownedRename: [],
    });
  });
});
