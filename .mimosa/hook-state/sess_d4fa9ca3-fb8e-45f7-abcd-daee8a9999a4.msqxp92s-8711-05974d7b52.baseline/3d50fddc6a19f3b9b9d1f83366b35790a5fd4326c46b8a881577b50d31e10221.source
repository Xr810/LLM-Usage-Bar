import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseUpgrade } from "@/components/DatabaseUpgrade";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  exit: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (
      _key: string,
      fallbackOrOptions?: string | { defaultValue?: string },
    ) =>
      typeof fallbackOrOptions === "string"
        ? fallbackOrOptions
        : (fallbackOrOptions?.defaultValue ?? _key),
  }),
}));

describe("DatabaseUpgrade", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("shows managed updates as unavailable without treating a check failure as upgradable", async () => {
    render(
      <DatabaseUpgrade
        payload={{ db_version: 14, supported_version: 13, path: "/tmp/db" }}
      />,
    );

    expect(
      screen.getByText("Managed updates are unavailable"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Upgrade app" }),
    ).not.toBeInTheDocument();

    await waitFor(() => expect(invokeMock).not.toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Open releases page" }));
    expect(invokeMock).toHaveBeenCalledWith("open_external", {
      url: "https://github.com/Xr810/LLM-Usage-Bar/releases",
    });
  });
});
