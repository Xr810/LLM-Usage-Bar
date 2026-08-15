import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { resolveWindowSurface, WindowSurface } from "./windowSurface";

vi.mock("./App", () => ({
  default: () => <div data-testid="main-app" />,
}));

vi.mock("@/components/tray-usage/TrayUsagePopover", () => ({
  TrayUsagePopover: () => <div data-testid="tray-usage-popover" />,
}));

describe("WindowSurface", () => {
  it("maps only the tray label to the popover material scope", () => {
    expect(resolveWindowSurface("tray-popover")).toBe("tray-popover");
    expect(resolveWindowSurface("main")).toBe("main-window");
    expect(resolveWindowSurface("future-main-label")).toBe("main-window");
  });

  it("renders only the popover surface for tray-popover", () => {
    render(<WindowSurface windowLabel="tray-popover" />);

    expect(screen.getByTestId("tray-usage-popover")).toBeInTheDocument();
    expect(screen.queryByTestId("main-app")).not.toBeInTheDocument();
  });

  it("renders the existing app for main and unknown labels", () => {
    const { rerender } = render(<WindowSurface windowLabel="main" />);

    expect(screen.getByTestId("main-app")).toBeInTheDocument();
    rerender(<WindowSurface windowLabel="future-main-label" />);
    expect(screen.getByTestId("main-app")).toBeInTheDocument();
  });
});
