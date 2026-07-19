import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageThresholdSettings } from "./UsageThresholdSettings";

const updateSettings = vi.fn();
const autoSaveSettings = vi.fn().mockResolvedValue({ requiresRestart: false });

vi.mock("@/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: {
      usageWarningRemainingPercent: 50,
      usageCriticalRemainingPercent: 20,
    },
    isLoading: false,
    isSaving: false,
    updateSettings,
    autoSaveSettings,
  }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key,
  }),
}));

describe("UsageThresholdSettings", () => {
  beforeEach(() => vi.clearAllMocks());

  it("persists a valid custom yellow and red band", async () => {
    render(<UsageThresholdSettings />);

    fireEvent.change(
      screen.getByRole("slider", { name: "Yellow at or below" }),
      {
        target: { value: "60" },
      },
    );
    fireEvent.change(screen.getByRole("slider", { name: "Red below" }), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save thresholds" }));

    const expected = {
      usageWarningRemainingPercent: 60,
      usageCriticalRemainingPercent: 30,
    };
    expect(updateSettings).toHaveBeenCalledWith(expected);
    await waitFor(() =>
      expect(autoSaveSettings).toHaveBeenCalledWith(expected),
    );
  });

  it("clamps the red slider when yellow is dragged below it", () => {
    render(<UsageThresholdSettings />);

    fireEvent.change(
      screen.getByRole("slider", { name: "Yellow at or below" }),
      {
        target: { value: "10" },
      },
    );

    expect(screen.getByRole("slider", { name: "Red below" })).toHaveValue("10");
    expect(screen.getAllByText("10%", { selector: "output" })).toHaveLength(2);
  });

  it("shows a local error when persistence fails", async () => {
    autoSaveSettings.mockRejectedValueOnce(new Error("save failed"));
    render(<UsageThresholdSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Save thresholds" }));

    expect(
      await screen.findByText("Could not save usage color thresholds."),
    ).toBeInTheDocument();
  });
});
