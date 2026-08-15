import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiBudgetSettings } from "./ApiBudgetSettings";

const mocks = vi.hoisted(() => ({
  mutateAsync: vi.fn(),
}));

vi.mock("@/lib/query/trayUsage", () => ({
  useSetApiBudgetConfig: () => ({
    mutateAsync: mocks.mutateAsync,
    isPending: false,
  }),
}));

describe("ApiBudgetSettings", () => {
  beforeEach(() => {
    mocks.mutateAsync.mockReset().mockResolvedValue({
      mode: "shared",
      sharedDailyBudgetUsd: "20",
    });
  });

  it("defaults to one shared limit and saves it without Provider identity", async () => {
    render(
      <ApiBudgetSettings
        config={{ mode: "shared", sharedDailyBudgetUsd: null }}
      />,
    );

    const input = screen.getByLabelText("Combined API daily limit (USD)");
    fireEvent.change(input, { target: { value: "20" } });
    fireEvent.click(screen.getByRole("button", { name: "Save budget" }));

    await waitFor(() =>
      expect(mocks.mutateAsync).toHaveBeenCalledWith({
        mode: "shared",
        sharedDailyBudgetUsd: "20",
      }),
    );
    expect(
      screen.getByText(/all enabled API Providers is added together/i),
    ).toBeInTheDocument();
  });

  it("enables independent Provider limits without discarding the shared value", async () => {
    render(
      <ApiBudgetSettings
        config={{ mode: "shared", sharedDailyBudgetUsd: "20" }}
      />,
    );

    fireEvent.click(
      screen.getByRole("switch", {
        name: "Set a separate limit for each Provider",
      }),
    );

    await waitFor(() =>
      expect(mocks.mutateAsync).toHaveBeenCalledWith({
        mode: "per_provider",
        sharedDailyBudgetUsd: "20",
      }),
    );
  });

  it("rejects a non-positive shared limit before invoking the backend", () => {
    render(
      <ApiBudgetSettings
        config={{ mode: "shared", sharedDailyBudgetUsd: null }}
      />,
    );

    fireEvent.change(screen.getByLabelText("Combined API daily limit (USD)"), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save budget" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Enter a positive amount.",
    );
    expect(mocks.mutateAsync).not.toHaveBeenCalled();
  });
});
