import { describe, expect, it } from "vitest";

import { toneFromStatus } from "./usagePresentation";

describe("toneFromStatus", () => {
  it("maps backend pace statuses to dashboard tones", () => {
    expect(toneFromStatus("green")).toBe("success");
    expect(toneFromStatus("yellow")).toBe("warning");
    expect(toneFromStatus("red")).toBe("danger");
    expect(toneFromStatus("unknown")).toBe("muted");
    expect(toneFromStatus(null)).toBe("muted");
    expect(toneFromStatus(undefined)).toBe("muted");
  });
});
