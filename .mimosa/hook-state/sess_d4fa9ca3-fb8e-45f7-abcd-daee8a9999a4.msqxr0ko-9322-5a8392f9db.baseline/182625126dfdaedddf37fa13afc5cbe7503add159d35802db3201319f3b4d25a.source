import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageDiagnosticsPanel } from "./UsageDiagnosticsPanel";

vi.mock("@/lib/query/usageDashboard", () => ({
  useUnassignedUsageDiagnostics: () => ({
    data: {
      unassignedEventCount: 4,
      unassignedGroups: [
        {
          providerId: "legacy-provider",
          source: "proxy",
          eventCount: 4,
          firstOccurredAt: 10,
          lastOccurredAt: 20,
        },
      ],
      archivedAgentHistory: [
        {
          agentModuleId: "custom-archived",
          eventCount: 3,
          firstOccurredAt: 30,
          lastOccurredAt: 40,
        },
      ],
      invalidLinkSummaries: [
        {
          reason: "cross_agent",
          linkCount: 2,
          firstCreatedAt: 50,
          lastCreatedAt: 60,
        },
      ],
    },
    isLoading: false,
    error: null,
  }),
}));

describe("UsageDiagnosticsPanel", () => {
  it("shows only safe aggregate ownership diagnostics", () => {
    render(<UsageDiagnosticsPanel />);

    expect(screen.getByText("4 unassigned events")).toBeInTheDocument();
    expect(screen.getByText("legacy-provider")).toBeInTheDocument();
    expect(screen.getByText("custom-archived")).toBeInTheDocument();
    expect(screen.getByText("cross_agent")).toBeInTheDocument();
    expect(screen.getByText("2 invalid links")).toBeInTheDocument();
    for (const unsafe of [
      "requestId",
      "sessionId",
      "correlation",
      "linkValue",
    ]) {
      expect(screen.queryByText(unsafe, { exact: false })).toBeNull();
    }
  });
});
