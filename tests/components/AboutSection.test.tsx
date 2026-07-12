import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const openExternalMock = vi.fn();
const getToolVersionsMock = vi.fn();

vi.mock("@/lib/api", () => ({
  settingsApi: {
    getToolVersions: (...args: unknown[]) => getToolVersionsMock(...args),
    openExternal: (...args: unknown[]) => openExternalMock(...args),
    probeToolInstallations: vi.fn(),
    runToolLifecycleAction: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(async () => "1.0.0"),
}));

vi.mock("@/lib/platform", () => ({
  isWindows: () => false,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

import { AboutSection } from "@/components/settings/AboutSection";

describe("AboutSection updater boundary", () => {
  beforeEach(() => {
    openExternalMock.mockReset();
    getToolVersionsMock.mockReset();
    getToolVersionsMock.mockResolvedValue([]);
  });

  it("shows managed updates as unavailable and opens only this repository's releases", async () => {
    render(<AboutSection isPortable={false} />);

    const unavailable = screen.getByRole("button", {
      name: "settings.managedUpdatesUnavailable",
    });
    expect(unavailable).toBeDisabled();

    await screen.findByText("v1.0.0");
    fireEvent.click(
      screen.getByRole("button", { name: "settings.releaseNotes" }),
    );

    await waitFor(() =>
      expect(openExternalMock).toHaveBeenCalledWith(
        "https://github.com/Xr810/LLM-Usage-Bar/releases/tag/v1.0.0",
      ),
    );
  });
});
