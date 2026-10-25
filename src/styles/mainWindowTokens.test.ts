import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

describe("main window tokens", () => {
  it("replaces the old accent with the v5 blue", () => {
    const css = readFileSync(
      path.resolve(process.cwd(), "src/index.css"),
      "utf8",
    );
    expect(css).not.toContain("243 75% 59%");
    expect(css).not.toContain("239 84% 68%");
    expect(css).not.toContain("243 70% 96%");
    expect(css).not.toContain("243 62% 42%");
    expect(css).not.toMatch(/Obsidian|indigo/i);
    expect(css).toContain("--primary: 211 100% 45%");
    expect(css).toContain("--primary: 211 100% 52%");
    expect(css).toContain("--ring: 211 100% 45%");
    expect(css).toContain("--ring: 211 100% 52%");
  });
});
