import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

vi.mock("@/components/usage-dashboard/UsageDashboardPage", () => ({
  UsageDashboardPage: () => <div data-testid="settings-usage-dashboard" />,
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ open, onOpenChange, children }: any) =>
    open ? (
      <div data-testid="dialog-root">
        {children}
        <button type="button" onClick={() => onOpenChange(false)}>
          close-dialog
        </button>
      </div>
    ) : null,
  DialogContent: ({ children }: any) => <div>{children}</div>,
  DialogHeader: ({ children }: any) => <div>{children}</div>,
  DialogTitle: ({ children }: any) => <h2>{children}</h2>,
  DialogDescription: ({ children }: any) => <div>{children}</div>,
}));

describe("SettingsPage usage-only surface", () => {
  it("renders only the usage dashboard while open", () => {
    render(<SettingsPage open onOpenChange={() => {}} defaultTab="advanced" />);
    expect(screen.getByTestId("settings-usage-dashboard")).toBeInTheDocument();
    expect(screen.queryByText("General")).not.toBeInTheDocument();
    expect(screen.queryByText("Advanced")).not.toBeInTheDocument();
  });

  it("does not render its content while closed", () => {
    render(<SettingsPage open={false} onOpenChange={() => {}} />);
    expect(
      screen.queryByTestId("settings-usage-dashboard"),
    ).not.toBeInTheDocument();
  });

  it("forwards close requests", () => {
    const onOpenChange = vi.fn();
    render(<SettingsPage open onOpenChange={onOpenChange} />);
    fireEvent.click(screen.getByRole("button", { name: "close-dialog" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
