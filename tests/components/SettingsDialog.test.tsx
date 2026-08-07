import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

vi.mock("@/components/settings/UsageProvidersSettings", () => ({
  UsageProvidersSettings: ({
    targetProviderId,
    onTargetHandled,
  }: {
    targetProviderId?: string;
    onTargetHandled?: () => void;
  }) => (
    <div>
      <button type="button">Add Provider</button>
      <output aria-label="Provider target">{targetProviderId ?? "none"}</output>
      <button type="button" onClick={onTargetHandled}>
        Handle provider target
      </button>
    </div>
  ),
}));

vi.mock("@/components/settings/UsageDiagnosticsPanel", () => ({
  UsageDiagnosticsPanel: () => <div>Aggregate diagnostics content</div>,
}));

vi.mock("@/components/settings/StartupSettings", () => ({
  StartupSettings: () => <div>Startup settings content</div>,
}));

vi.mock("@/components/settings/UsageLightInfo", () => ({
  UsageLightInfo: () => <div>Usage light info content</div>,
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ open, children }: { open: boolean; children: ReactNode }) =>
    open ? <div data-testid="dialog-root">{children}</div> : null,
  DialogContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogClose: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  DialogHeader: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
  DialogDescription: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
}));

describe("SettingsPage Provider-only sections", () => {
  it("defaults unknown historical tabs to General and keeps Provider tools available", async () => {
    const user = userEvent.setup();
    render(<SettingsPage open onOpenChange={() => {}} defaultTab="advanced" />);

    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("Startup settings content")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Agents" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Proxy setup" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(
      screen.getByText("Aggregate diagnostics content"),
    ).toBeInTheDocument();
  });

  it.each(["agents", "modules", "proxy", "unknown"])(
    "maps legacy default %s to General",
    (defaultTab) => {
      render(
        <SettingsPage open onOpenChange={() => {}} defaultTab={defaultTab} />,
      );
      expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    },
  );

  it("opens Startup for the general section", () => {
    render(<SettingsPage open onOpenChange={() => {}} defaultTab="general" />);
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("preserves a Provider target until the Provider DOM handles it", async () => {
    const user = userEvent.setup();
    const onProviderTargetHandled = vi.fn();
    render(
      <SettingsPage
        open
        onOpenChange={() => {}}
        defaultTab="providers"
        defaultProviderId="system-openrouter-api"
        onProviderTargetHandled={onProviderTargetHandled}
      />,
    );

    expect(screen.getByLabelText("Provider target")).toHaveTextContent(
      "system-openrouter-api",
    );
    await user.click(
      screen.getByRole("button", { name: "Handle provider target" }),
    );
    expect(screen.getByLabelText("Provider target")).toHaveTextContent("none");
    expect(onProviderTargetHandled).toHaveBeenCalledOnce();
  });

  it("does not mount sections while closed and keeps an accessible close control", () => {
    const { rerender } = render(
      <SettingsPage open={false} onOpenChange={() => {}} />,
    );
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    rerender(<SettingsPage open onOpenChange={() => {}} />);
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });
});
