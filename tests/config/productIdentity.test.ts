import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parse as parseToml } from "smol-toml";

describe("product identity manifests", () => {
  it("uses the LLM Usage Bar package and bundle identity", () => {
    const pkg = JSON.parse(readFileSync("package.json", "utf8"));
    const cargo = parseToml(readFileSync("src-tauri/Cargo.toml", "utf8"));
    const tauri = JSON.parse(
      readFileSync("src-tauri/tauri.conf.json", "utf8"),
    );

    expect(pkg.name).toBe("llm-usage-bar");
    expect(cargo.package.name).toBe("llm-usage-bar");
    expect(cargo.lib.name).toBe("llm_usage_bar_lib");
    expect(tauri.productName).toBe("LLM Usage Bar");
    expect(tauri.identifier).toBe("com.llmusagebar.desktop");
  });
});
