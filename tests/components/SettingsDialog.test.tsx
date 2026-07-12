import type { ReactNode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

vi.mock("@/components/settings/DashboardModulesSettings", () => ({
  DashboardModulesSettings: () => <div>Module settings content</div>,
}));

vi.mock("@/components/settings/UsageProvidersSettings", () => ({
  UsageProvidersSettings: () => <button type="button">Add Provider</button>,
}));

vi.mock("@/components/settings/ProxyRoutingSettings", () => ({
  ProxyRoutingSettings: () => <button type="button">Start proxy</button>,
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({
    open,
    onOpenChange,
    children,
  }: {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    children: ReactNode;
  }) =>
    open ? (
      <div data-testid="dialog-root">
        {children}
        <button type="button" onClick={() => onOpenChange(false)}>
          close-dialog
        </button>
      </div>
    ) : null,
  DialogContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
  DialogDescription: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
}));

describe("SettingsPage sections", () => {
  it("defaults historical values to Usage modules and isolates controls by tab", async () => {
    const user = userEvent.setup();
    render(<SettingsPage open onOpenChange={() => {}} defaultTab="advanced" />);

    expect(
      screen.getByRole("heading", { name: "Settings" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Usage modules" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("Module settings content")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Provider" }));
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Module settings content")).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Proxy & routing" }));
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
  });

  it.each([
    ["modules", "Usage modules"],
    ["providers", "Provider"],
    ["proxy", "Proxy & routing"],
  ])("honors the valid %s default", (defaultTab, selectedName) => {
    render(
      <SettingsPage open onOpenChange={() => {}} defaultTab={defaultTab} />,
    );
    expect(screen.getByRole("tab", { name: selectedName })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("does not mount settings sections while closed", () => {
    render(<SettingsPage open={false} onOpenChange={() => {}} />);
    expect(screen.queryByText("Module settings content")).toBeNull();
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();
  });

  it("forwards close requests", () => {
    const onOpenChange = vi.fn();
    render(<SettingsPage open onOpenChange={onOpenChange} />);
    fireEvent.click(screen.getByRole("button", { name: "close-dialog" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
