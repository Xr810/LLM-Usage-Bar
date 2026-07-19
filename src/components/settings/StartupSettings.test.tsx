import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StartupSettings } from "./StartupSettings";

const updateSettings = vi.fn();
const autoSaveSettings = vi.fn();

vi.mock("@/hooks/useSettings", () => ({
  useSettings: vi.fn(() => ({
    settings: { launchOnStartup: false, silentStartup: false },
    isLoading: false,
    isSaving: false,
    updateSettings,
    autoSaveSettings,
  })),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key,
  }),
}));

describe("StartupSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("updates and persists launch-on-startup when toggled", () => {
    render(<StartupSettings />);

    fireEvent.click(
      screen.getByRole("switch", { name: "settings.launchOnStartup" }),
    );

    expect(updateSettings).toHaveBeenCalledWith({ launchOnStartup: true });
    expect(autoSaveSettings).toHaveBeenCalledWith({ launchOnStartup: true });
  });
});
