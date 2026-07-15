import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";

const mocks = vi.hoisted(() => ({
  login: vi.fn(),
  logout: vi.fn(),
  refetch: vi.fn(),
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useClaudeCliAuthStatus: () => ({
    data: {
      installed: true,
      authenticated: true,
      subscriptionType: "max",
      quotaAvailability: "unavailable",
      errorCode: null,
    },
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
  it("shows official CLI state, unavailable quota, reconnect, and logout", async () => {
    render(<ClaudeCliAuthSection />);
    expect(screen.getByText("Connected · Max")).toBeInTheDocument();
    expect(screen.getByText("Quota unavailable")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reconnect Claude" }));
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Claude" }));
    await waitFor(() => expect(mocks.login).toHaveBeenCalledOnce());
    expect(mocks.logout).toHaveBeenCalledOnce();
  });
});
