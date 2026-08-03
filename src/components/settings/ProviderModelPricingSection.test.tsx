import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderModelPricingSection } from "./ProviderModelPricingSection";
import type { ProviderModelPricingView } from "@/types/usageDashboard";

const state = vi.hoisted(() => ({
  rows: [] as ProviderModelPricingView[],
  isLoading: false,
  update: { mutateAsync: vi.fn(), isPending: false },
  remove: { mutateAsync: vi.fn(), isPending: false },
}));

vi.mock("@/lib/query/usage", () => ({
  useProviderModelPricing: () => ({
    data: state.rows,
    isLoading: state.isLoading,
  }),
  useUpdateProviderModelPricing: () => state.update,
  useDeleteProviderModelPricing: () => state.remove,
}));

function row(
  overrides: Partial<ProviderModelPricingView> = {},
): ProviderModelPricingView {
  return {
    providerId: "relay-a",
    modelId: "claude-sonnet-5",
    displayName: "claude-sonnet-5",
    inputCostPerMillion: "1.5",
    outputCostPerMillion: "7.5",
    cacheReadCostPerMillion: "0.15",
    cacheCreationCostPerMillion: "1.875",
    updatedAt: 1,
    ...overrides,
  };
}

function renderSection() {
  return render(
    <ProviderModelPricingSection providerId="relay-a" providerName="Relay A" />,
  );
}

async function expand(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    screen.getByRole("button", { name: /relay a model prices/i }),
  );
}

describe("ProviderModelPricingSection", () => {
  beforeEach(() => {
    state.rows = [];
    state.isLoading = false;
    state.update.mutateAsync.mockReset().mockResolvedValue(undefined);
    state.update.isPending = false;
    state.remove.mutateAsync.mockReset().mockResolvedValue(undefined);
    state.remove.isPending = false;
  });

  it("submits the four rates as decimal strings without rounding them", async () => {
    const user = userEvent.setup();
    renderSection();
    await expand(user);

    await user.type(screen.getByLabelText(/model id/i), "claude-sonnet-5");
    await user.clear(screen.getByLabelText(/^input/i));
    await user.type(screen.getByLabelText(/^input/i), "1.5");
    await user.clear(screen.getByLabelText(/^output/i));
    await user.type(screen.getByLabelText(/^output/i), "7.5");
    await user.clear(screen.getByLabelText(/cache read/i));
    await user.type(screen.getByLabelText(/cache read/i), "0.15");
    await user.clear(screen.getByLabelText(/cache write/i));
    await user.type(screen.getByLabelText(/cache write/i), "1.875");
    await user.click(screen.getByRole("button", { name: /add price/i }));

    await waitFor(() => {
      expect(state.update.mutateAsync).toHaveBeenCalledWith({
        providerId: "relay-a",
        modelId: "claude-sonnet-5",
        displayName: "claude-sonnet-5",
        price: {
          inputCostPerMillion: "1.5",
          outputCostPerMillion: "7.5",
          cacheReadCostPerMillion: "0.15",
          cacheCreationCostPerMillion: "1.875",
        },
      });
    });
  });

  it("rejects a missing model id before calling the backend", async () => {
    const user = userEvent.setup();
    renderSection();
    await expand(user);

    await user.clear(screen.getByLabelText(/^input/i));
    await user.type(screen.getByLabelText(/^input/i), "1");
    await user.clear(screen.getByLabelText(/^output/i));
    await user.type(screen.getByLabelText(/^output/i), "2");
    await user.click(screen.getByRole("button", { name: /add price/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/model id/i);
    expect(state.update.mutateAsync).not.toHaveBeenCalled();
  });

  it("rejects a negative rate before calling the backend", async () => {
    const user = userEvent.setup();
    renderSection();
    await expand(user);

    await user.type(screen.getByLabelText(/model id/i), "some-model");
    await user.clear(screen.getByLabelText(/^input/i));
    await user.type(screen.getByLabelText(/^input/i), "-1");
    await user.clear(screen.getByLabelText(/^output/i));
    await user.type(screen.getByLabelText(/^output/i), "2");
    await user.click(screen.getByRole("button", { name: /add price/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/non-negative/i);
    expect(state.update.mutateAsync).not.toHaveBeenCalled();
  });

  it("loads an existing row into the form and keeps its model id fixed", async () => {
    const user = userEvent.setup();
    state.rows = [row()];
    renderSection();
    await expand(user);

    await user.click(screen.getByRole("button", { name: /^edit$/i }));

    const modelInput = screen.getByLabelText(/model id/i);
    expect(modelInput).toHaveValue("claude-sonnet-5");
    expect(modelInput).toBeDisabled();
    expect(screen.getByLabelText(/^input/i)).toHaveValue(1.5);
    expect(screen.getByLabelText(/cache write/i)).toHaveValue(1.875);
  });

  it("deletes one model price by id", async () => {
    const user = userEvent.setup();
    state.rows = [row(), row({ modelId: "claude-opus-5" })];
    renderSection();
    await expand(user);

    await user.click(
      screen.getByRole("button", {
        name: /delete custom price for claude-opus-5/i,
      }),
    );

    await waitFor(() => {
      expect(state.remove.mutateAsync).toHaveBeenCalledWith({
        providerId: "relay-a",
        modelId: "claude-opus-5",
      });
    });
  });

  it("surfaces a backend rejection instead of silently succeeding", async () => {
    const user = userEvent.setup();
    state.update.mutateAsync.mockRejectedValue(
      new Error("Subscription plans are valued at official list prices"),
    );
    renderSection();
    await expand(user);

    await user.type(screen.getByLabelText(/model id/i), "claude-sonnet-5");
    await user.clear(screen.getByLabelText(/^input/i));
    await user.type(screen.getByLabelText(/^input/i), "1");
    await user.clear(screen.getByLabelText(/^output/i));
    await user.type(screen.getByLabelText(/^output/i), "2");
    await user.click(screen.getByRole("button", { name: /add price/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /official list prices/i,
    );
  });

  it("says costs fall back to reported or official pricing when nothing is priced", () => {
    renderSection();

    expect(
      screen.getByText(/costs use the reported or official price/i),
    ).toBeInTheDocument();
  });
});
