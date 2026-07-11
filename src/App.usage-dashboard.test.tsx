import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { SettingsPage } from "@/components/settings/SettingsPage";

vi.mock("@/components/usage-dashboard/UsageDashboardPage", () => ({
  UsageDashboardPage: () => (
    <div data-testid="usage-dashboard-main">
      <h1>Provider-aware usage dashboard</h1>
      <section>Provider routes</section>
      <button type="button">Start proxy</button>
    </div>
  ),
}));

function renderWithQuery(ui: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>{ui}</QueryClientProvider>,
  );
}

const legacyLabels = [
  "Quick switching",
  "Failover",
  "Preset marketplace",
  "MCP",
  "Skills",
  "OpenClaw",
  "WebDAV",
  "S3",
];

describe("usage dashboard main path", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders dashboard, provider routes and proxy controls without legacy entry points", () => {
    renderWithQuery(<App />);

    expect(screen.getByTestId("usage-dashboard-main")).toBeInTheDocument();
    expect(screen.getByText("Provider routes")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
    for (const label of legacyLabels) {
      expect(
        screen.queryByText(label, { exact: false }),
      ).not.toBeInTheDocument();
    }
  });

  it("opens the usage-only settings surface", () => {
    renderWithQuery(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getAllByTestId("usage-dashboard-main")).toHaveLength(2);
    for (const label of legacyLabels) {
      expect(
        screen.queryByText(label, { exact: false }),
      ).not.toBeInTheDocument();
    }
  });

  it.each(["general", "proxy", "auth", "advanced", "mcp", "skills", "about"])(
    "falls back legacy default tab %s to usage",
    (defaultTab) => {
      renderWithQuery(
        <SettingsPage open onOpenChange={() => {}} defaultTab={defaultTab} />,
      );
      expect(screen.getByTestId("usage-dashboard-main")).toBeInTheDocument();
    },
  );
});
