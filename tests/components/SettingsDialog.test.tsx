import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

vi.mock("@/components/settings/DashboardModulesSettings", () => ({
  AgentsSettings: () => <div>Agent settings content</div>,
}));

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

vi.mock("@/components/settings/ProxyRoutingSettings", () => ({
  ProxyRoutingSettings: () => <button type="button">Start proxy</button>,
}));

vi.mock("@/components/settings/UsageDiagnosticsPanel", () => ({
  UsageDiagnosticsPanel: () => <div>Aggregate diagnostics content</div>,
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

describe("SettingsPage sections", () => {
  it("defaults historical values to Agents and isolates all four sections", async () => {
    const user = userEvent.setup();
    render(<SettingsPage open onOpenChange={() => {}} defaultTab="advanced" />);

    expect(
      screen.getByRole("heading", { name: "Settings" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByText("Agent settings content")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Agent settings content")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Proxy setup" }));
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(
      screen.getByText("Aggregate diagnostics content"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();
  });

  it.each([
    ["agents", "Agents"],
    ["modules", "Agents"],
    ["providers", "Providers"],
    ["proxy", "Proxy setup"],
    ["diagnostics", "Diagnostics"],
  ])(
    "honors the valid or compatible %s default",
    (defaultTab, selectedName) => {
      render(
        <SettingsPage open onOpenChange={() => {}} defaultTab={defaultTab} />,
      );
      expect(screen.getByRole("tab", { name: selectedName })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    },
  );

  it("does not mount settings sections while closed", () => {
    render(<SettingsPage open={false} onOpenChange={() => {}} />);
    expect(screen.queryByText("Agent settings content")).toBeNull();
    expect(screen.queryByRole("button", { name: "Add Provider" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start proxy" })).toBeNull();
    expect(screen.queryByText("Aggregate diagnostics content")).toBeNull();
  });

  it("renders an accessible close control", () => {
    render(<SettingsPage open onOpenChange={() => {}} />);
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
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

    await user.click(screen.getByRole("tab", { name: "Agents" }));
    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(screen.getByLabelText("Provider target")).toHaveTextContent(
      "system-openrouter-api",
    );

    await user.click(
      screen.getByRole("button", { name: "Handle provider target" }),
    );
    expect(screen.getByLabelText("Provider target")).toHaveTextContent("none");
    expect(onProviderTargetHandled).toHaveBeenCalledOnce();
  });
});
