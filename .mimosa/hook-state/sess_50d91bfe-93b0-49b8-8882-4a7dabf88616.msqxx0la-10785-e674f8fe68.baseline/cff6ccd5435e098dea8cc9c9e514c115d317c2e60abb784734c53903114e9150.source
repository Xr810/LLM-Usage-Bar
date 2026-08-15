import { describe, expect, it } from "vitest";
import {
  agentLabel,
  costText,
  formatSharePercent,
  modelLabel,
  productGroupLabel,
  rowCostStatus,
  sharePercent,
} from "./breakdownPresentation";

/** Stand-in for i18next: returns the supplied default. */
const t = (_key: string, options?: { defaultValue?: string }) =>
  options?.defaultValue ?? "";

describe("sharePercent", () => {
  it("returns the percentage of the range total", () => {
    expect(sharePercent(25, 100)).toBe(25);
  });

  it("treats an empty or invalid range as zero rather than dividing", () => {
    expect(sharePercent(10, 0)).toBe(0);
    expect(sharePercent(10, -5)).toBe(0);
    expect(sharePercent(Number.NaN, 100)).toBe(0);
  });

  it("clamps a part that exceeds the total", () => {
    expect(sharePercent(150, 100)).toBe(100);
  });
});

describe("formatSharePercent", () => {
  it("keeps one decimal below ten percent and rounds above it", () => {
    expect(formatSharePercent(4.25)).toBe("4.3%");
    expect(formatSharePercent(42.4)).toBe("42%");
  });

  it("marks a non-zero sliver instead of rounding it away", () => {
    expect(formatSharePercent(0.04)).toBe("<0.1%");
    expect(formatSharePercent(0)).toBe("0%");
  });
});

describe("productGroupLabel", () => {
  it("names the built-in plans", () => {
    expect(productGroupLabel("claude-subscription", [], t)).toBe(
      "Claude Pro/Max",
    );
    expect(productGroupLabel("chatgpt-subscription", [], t)).toBe(
      "ChatGPT Plus/Pro",
    );
  });

  it("falls back to the accounts in a custom group", () => {
    expect(productGroupLabel("my-group", ["Relay A", "Relay B"], t)).toBe(
      "Relay A · Relay B",
    );
    expect(productGroupLabel("my-group", ["Relay A"], t)).toBe("Relay A");
  });

  it("shows the raw group id when nothing else identifies it", () => {
    expect(productGroupLabel("my-group", ["  "], t)).toBe("my-group");
  });
});

describe("rowCostStatus", () => {
  const counts = (
    upstream: number,
    estimated: number,
    unavailable: number,
  ) => ({ upstream, estimated, unavailable });

  it("reports an absent total as unavailable", () => {
    expect(rowCostStatus(null, counts(0, 0, 3))).toBe("unavailable");
  });

  it("downgrades a known total that still has unpriced events", () => {
    expect(rowCostStatus("1.00", counts(5, 0, 1))).toBe("partial");
  });

  it("calls out estimated pricing separately from a trusted total", () => {
    expect(rowCostStatus("1.00", counts(5, 2, 0))).toBe("estimated");
    expect(rowCostStatus("1.00", counts(5, 0, 0))).toBe("complete");
  });
});

describe("labels", () => {
  it("renders a cost, or says it is unavailable", () => {
    expect(costText("1.25", t)).toBe("$1.25");
    expect(costText(null, t)).toBe("Cost unavailable");
  });

  it("keeps a model id verbatim and names the empty case", () => {
    expect(modelLabel("claude-opus-5", t)).toBe("claude-opus-5");
    expect(modelLabel("  ", t)).toBe("Unknown model");
    expect(modelLabel("unknown", t)).toBe("Unknown model");
  });

  it("labels agents, falling back to id then to the unassigned bucket", () => {
    expect(agentLabel("codex", "Codex", t)).toBe("Codex");
    expect(agentLabel("gone", null, t)).toBe("gone");
    expect(agentLabel(null, null, t)).toBe("Unassigned");
  });
});
