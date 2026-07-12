import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProxyRoutingSettings } from "./ProxyRoutingSettings";

const mocks = vi.hoisted(() => ({
  running: false,
  start: vi.fn(),
  stop: vi.fn(),
  setBinding: vi.fn(),
}));

vi.mock("@/lib/query/proxy", () => ({
  useIsProxyRunning: () => ({
    data: mocks.running,
    isLoading: false,
    error: null,
  }),
  useStartProxyServer: () => ({
    mutateAsync: mocks.start,
    isPending: false,
  }),
  useStopProxyServer: () => ({
    mutateAsync: mocks.stop,
    isPending: false,
  }),
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageProviders: () => ({
    data: [{ id: "metered", name: "Metered API" }],
    isLoading: false,
    error: null,
  }),
  useRouteBindings: () => ({
    data: [{ protocol: "codex", providerId: "metered", updatedAt: 1 }],
    isLoading: false,
    error: null,
  }),
  useSetRouteBinding: () => ({
    mutateAsync: mocks.setBinding,
    isPending: false,
  }),
}));

vi.mock("@/components/usage-dashboard/RouteBindingsPanel", () => ({
  RouteBindingsPanel: ({
    title,
    description,
    providers,
    bindings,
    onSave,
  }: {
    title: string;
    description: string;
    providers: Array<{ id: string }>;
    bindings: Array<{ protocol: string }>;
    onSave: (protocol: string, providerId: string) => Promise<unknown>;
  }) => (
    <div>
      <h3>{title}</h3>
      <p>{description}</p>
      <span>
        Providers {providers.map((provider) => provider.id).join(",")}
      </span>
      <span>
        Bindings {bindings.map((binding) => binding.protocol).join(",")}
      </span>
      <button type="button" onClick={() => void onSave("codex", "metered")}>
        Save Codex target
      </button>
    </div>
  ),
}));

describe("ProxyRoutingSettings", () => {
  beforeEach(() => {
    mocks.running = false;
    mocks.start.mockReset().mockResolvedValue(undefined);
    mocks.stop.mockReset().mockResolvedValue(undefined);
    mocks.setBinding.mockReset().mockResolvedValue(undefined);
  });

  it("starts a stopped proxy and explains forwarding targets", async () => {
    render(<ProxyRoutingSettings />);

    expect(screen.getByText("Proxy stopped")).toBeInTheDocument();
    expect(screen.getByText("Proxy forwarding targets")).toBeInTheDocument();
    expect(
      screen.getByText(
        "These targets decide where requests are forwarded. They are not a dashboard filter.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Providers metered")).toBeInTheDocument();
    expect(screen.getByText("Bindings codex")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Start proxy" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledOnce());
  });

  it("stops a running proxy", async () => {
    mocks.running = true;
    render(<ProxyRoutingSettings />);

    expect(screen.getByText("Proxy running")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stop proxy" }));
    await waitFor(() => expect(mocks.stop).toHaveBeenCalledOnce());
  });

  it("saves RouteBindings through the settings-only mutation", async () => {
    render(<ProxyRoutingSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Save Codex target" }));
    await waitFor(() =>
      expect(mocks.setBinding).toHaveBeenCalledWith({
        protocol: "codex",
        providerId: "metered",
      }),
    );
  });

  it("surfaces proxy control failures", async () => {
    mocks.start.mockRejectedValueOnce(new Error("proxy unavailable"));
    render(<ProxyRoutingSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Start proxy" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "proxy unavailable",
    );
  });
});
