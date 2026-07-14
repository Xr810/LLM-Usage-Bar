import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentProxySetupPanel } from "./AgentProxySetupPanel";

vi.mock("@/lib/query/usageDashboard", () => ({
  useAgentProxySetupInfo: () => ({
    data: {
      agentModuleId: "claude-code",
      proxyRunning: true,
      proxyOrigin: "http://127.0.0.1:15721",
      routes: [
        {
          bindingId: "binding-secret-id",
          providerId: "anthropic",
          protocol: "claude-desktop",
          localBaseUrl: "http://127.0.0.1:15721/claude-desktop",
          credentialPlacements: ["x-api-key"],
          credentialStatus: "configured",
          upstreamUrl: "https://unsafe-upstream.example",
          fingerprint: "unsafe-fingerprint",
          slot: "unsafe-slot",
          maskedKeyHint: "sk-...unsafe",
        },
        {
          bindingId: "binding-no-key",
          providerId: "official-subscription",
          protocol: null,
          localBaseUrl: null,
          credentialPlacements: [],
          credentialStatus: "not_required",
        },
      ],
    },
    isLoading: false,
    error: null,
  }),
}));

describe("AgentProxySetupPanel", () => {
  it("renders only the safe Agent-scoped setup projection", () => {
    render(
      <AgentProxySetupPanel
        agent={{
          id: "claude-code",
          name: "Claude Code",
          sortOrder: 1,
          visible: true,
          isFixed: true,
          archivedAt: null,
          providerCount: 1,
        }}
      />,
    );

    expect(screen.getByText("Claude Code setup")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:15721")).toBeInTheDocument();
    expect(screen.getByText("anthropic")).toBeInTheDocument();
    expect(screen.getByText("Configured")).toBeInTheDocument();
    expect(screen.getAllByText("x-api-key", { exact: false })).not.toHaveLength(
      0,
    );
    expect(
      screen.getByText(/Claude Desktop: use x-api-key for the binding key/),
    ).toBeInTheDocument();
    const noKeyRoute = screen
      .getByText("official-subscription")
      .closest("div.grid");
    if (!(noKeyRoute instanceof HTMLElement)) {
      throw new Error("Expected an Agent proxy route container");
    }
    expect(
      within(noKeyRoute).getByText("No API key required"),
    ).toBeInTheDocument();

    for (const unsafe of [
      "binding-secret-id",
      "https://unsafe-upstream.example",
      "unsafe-fingerprint",
      "unsafe-slot",
      "sk-...unsafe",
    ]) {
      expect(screen.queryByText(unsafe, { exact: false })).toBeNull();
    }
  });
});
