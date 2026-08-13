import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageLightInfo } from "./UsageLightInfo";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key,
  }),
}));

describe("UsageLightInfo", () => {
  it("explains what each colour means", () => {
    render(<UsageLightInfo />);

    for (const [label, body] of [
      ["Green", "Projected to still have quota left when the window resets."],
      ["Yellow", "Projected to run out right around the reset."],
      ["Red", "Projected to run out before the window resets."],
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
      expect(screen.getByText(`— ${body}`)).toBeInTheDocument();
    }
  });

  it("documents the two cases where pace alone does not decide", () => {
    render(<UsageLightInfo />);

    expect(screen.getByText(/whatever the pace/)).toBeInTheDocument();
    expect(
      screen.getByText(/no clock to project against/i),
    ).toBeInTheDocument();
  });

  it("offers no controls — the verdict is not a matter of taste", () => {
    render(<UsageLightInfo />);

    expect(screen.queryAllByRole("slider")).toHaveLength(0);
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });
});
