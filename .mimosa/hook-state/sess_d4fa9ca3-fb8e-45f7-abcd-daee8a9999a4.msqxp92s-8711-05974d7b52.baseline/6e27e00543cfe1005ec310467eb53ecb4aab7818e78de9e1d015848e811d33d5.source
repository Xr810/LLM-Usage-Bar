import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OfficialPricingRefreshSection } from "./OfficialPricingRefreshSection";

const state = vi.hoisted(() => ({
  freshness: {
    data: 1_753_872_000 as number | null,
    isLoading: false,
    error: null as Error | null,
  },
  refresh: {
    mutateAsync: vi.fn(),
    isPending: false,
    error: null as Error | null,
  },
  importedCount: {
    data: null as number | null,
    isLoading: false,
    error: null as Error | null,
  },
}));

vi.mock("@/lib/query/usage", () => ({
  useOfficialPricingLastRefreshAt: () => state.freshness,
  useOfficialPricingLastImportedCount: () => state.importedCount,
  useRefreshOfficialPricing: () => state.refresh,
}));

describe("OfficialPricingRefreshSection", () => {
  beforeEach(() => {
    state.freshness.data = 1_753_872_000;
    state.freshness.isLoading = false;
    state.freshness.error = null;
    state.refresh.mutateAsync.mockReset().mockResolvedValue({
      fetchedAt: 1_754_000_000,
      modelsImported: 42,
      modelsSkipped: 7,
      sourceUrl: "https://models.dev/api.json",
    });
    state.refresh.isPending = false;
    state.refresh.error = null;
    state.importedCount.data = null;
    state.importedCount.isLoading = false;
    state.importedCount.error = null;
  });

  it("shows stored freshness and imported count and triggers a manual refresh", async () => {
    const user = userEvent.setup();
    state.importedCount.data = 42;
    render(<OfficialPricingRefreshSection />);

    expect(screen.getByText("Last refresh")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Refresh now" }));

    await waitFor(() =>
      expect(state.refresh.mutateAsync).toHaveBeenCalledOnce(),
    );
    expect(screen.getByText("42")).toBeInTheDocument();
  });

  it("surfaces the last refresh failure", () => {
    state.refresh.error = new Error("models.dev unavailable");
    render(<OfficialPricingRefreshSection />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "models.dev unavailable",
    );
  });
});
