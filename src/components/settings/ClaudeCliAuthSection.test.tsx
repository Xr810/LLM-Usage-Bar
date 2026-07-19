import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";

const mocks = vi.hoisted(() => ({
  login: vi.fn(),
  logout: vi.fn(),
  refetch: vi.fn(),
  statusData: {
    installed: true,
    authenticated: true,
    authMethod: "api_key" as
      | "api_key"
      | "claude_account"
      | "other"
      | null,
    subscriptionType: null as "pro" | "max" | null,
    quotaAvailability: "unavailable" as const,
    errorCode: null,
  },
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useClaudeCliAuthStatus: () => ({
    data: mocks.statusData,
    isLoading: false,
    refetch: mocks.refetch,
  }),
  useClaudeCliAuthActions: () => ({
    startLogin: mocks.login,
    logout: mocks.logout,
    isPending: false,
  }),
}));

describe("ClaudeCliAuthSection", () => {
  beforeEach(() => {
    mocks.statusData.authMethod = "api_key";
    mocks.statusData.subscriptionType = null;
  });

  it("shows detected API key CLI state, unavailable quota, reconnect, and logout", async () => {
    render(<ClaudeCliAuthSection />);
    expect(
      screen.getByText("Claude CLI detected · API Key"),
    ).toBeInTheDocument();
    expect(screen.getByText("Quota unavailable")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reconnect Claude" }));
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Claude" }));
    await waitFor(() => expect(mocks.login).toHaveBeenCalledOnce());
    expect(mocks.logout).toHaveBeenCalledOnce();
  });

  it("prefers the official subscription plan when one is available", () => {
    mocks.statusData.authMethod = "claude_account";
    mocks.statusData.subscriptionType = "max";
    render(<ClaudeCliAuthSection />);
    expect(screen.getByText("Claude CLI detected · Max")).toBeInTheDocument();
  });
});
