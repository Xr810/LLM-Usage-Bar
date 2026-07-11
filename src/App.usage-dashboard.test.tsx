import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { setSettings } from "../tests/msw/state";

const windowMocks = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowMocks,
}));

function renderApp() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
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
  beforeEach(() => {
    Object.values(windowMocks).forEach((mock) => mock.mockReset());
  });

  it("renders the real dashboard and no legacy business entry points", async () => {
    renderApp();

    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getAllByText("Metered API")).not.toHaveLength(0);
    expect(screen.getByText("Static routes")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Start proxy" }),
    ).toBeInTheDocument();
    for (const label of legacyLabels) {
      expect(
        screen.queryByText(label, { exact: false }),
      ).not.toBeInTheDocument();
    }
  });

  it("keeps a draggable title area and gates native window controls by settings", async () => {
    setSettings({ useAppWindowControls: true, language: "en" });
    const { container } = renderApp();

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Minimize window" }),
      ).toBeInTheDocument(),
    );
    const dragRegion = container.querySelector("[data-tauri-drag-region]");
    expect(dragRegion).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Minimize window" }));
    fireEvent.click(screen.getByRole("button", { name: "Maximize window" }));
    fireEvent.click(screen.getByRole("button", { name: "Close window" }));
    expect(windowMocks.minimize).toHaveBeenCalledOnce();
    expect(windowMocks.toggleMaximize).toHaveBeenCalledOnce();
    expect(windowMocks.close).toHaveBeenCalledOnce();
  });
});
