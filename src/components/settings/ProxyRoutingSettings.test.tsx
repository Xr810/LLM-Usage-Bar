import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProxyRoutingSettings } from "./ProxyRoutingSettings";

const mocks = vi.hoisted(() => ({
  running: false,
  start: vi.fn(),
  stop: vi.fn(),
  selectedAgentIds: [] as string[],
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
  useAgentModules: () => ({
    data: [
      {
        id: "codex",
        name: "Codex",
        sortOrder: 0,
        visible: true,
        isFixed: true,
      },
      {
        id: "claude-code",
        name: "Claude Code",
        sortOrder: 1,
        visible: true,
        isFixed: true,
      },
    ],
    isLoading: false,
    error: null,
  }),
  useAgentProxySetupInfo: (agentModuleId: string) => {
    mocks.selectedAgentIds.push(agentModuleId);
    return {
      data: {
        agentModuleId,
        proxyRunning: mocks.running,
        proxyOrigin: "http://127.0.0.1:15721",
        routes: [
          {
            bindingId: `binding-${agentModuleId}`,
            providerId: "metered",
            protocol: agentModuleId === "claude-code" ? "claude" : "codex",
            localBaseUrl: `http://127.0.0.1:15721/${agentModuleId}`,
            credentialPlacements: ["x-api-key"],
            credentialStatus: "configured",
          },
        ],
      },
      isLoading: false,
      error: null,
    };
  },
}));

describe("ProxyRoutingSettings", () => {
  beforeEach(() => {
    mocks.running = false;
    mocks.start.mockReset().mockResolvedValue(undefined);
    mocks.stop.mockReset().mockResolvedValue(undefined);
    mocks.selectedAgentIds.length = 0;
  });

  it("switches the read-only Agent setup without proxy or config mutations", async () => {
    render(<ProxyRoutingSettings />);

    expect(screen.getByText("Proxy stopped")).toBeInTheDocument();
    expect(screen.getByText("Codex setup")).toBeInTheDocument();
    expect(
      screen.getByText("The app does not edit Agent configuration."),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "Agent" }), {
      target: { value: "claude-code" },
    });

    expect(await screen.findByText("Claude Code setup")).toBeInTheDocument();
    expect(screen.getAllByText("x-api-key", { exact: false })).not.toHaveLength(
      0,
    );
    expect(
      screen.getByText("gateway token", { exact: false }),
    ).toBeInTheDocument();
    expect(mocks.start).not.toHaveBeenCalled();
    expect(mocks.stop).not.toHaveBeenCalled();
  });

  it("keeps start and stop as explicit user controls", async () => {
    const { rerender } = render(<ProxyRoutingSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Start proxy" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledOnce());

    mocks.running = true;
    rerender(<ProxyRoutingSettings />);
    fireEvent.click(screen.getByRole("button", { name: "Stop proxy" }));
    await waitFor(() => expect(mocks.stop).toHaveBeenCalledOnce());
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
