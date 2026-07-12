import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SkillStorageLocationSettings } from "@/components/settings/SkillStorageLocationSettings";

const { migrateStorage } = vi.hoisted(() => ({
  migrateStorage: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/lib/api/skills", () => ({
  skillsApi: { migrateStorage },
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ open, children }: { open: boolean; children: React.ReactNode }) =>
    open ? <>{children}</> : null,
  DialogContent: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogDescription: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogFooter: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
}));

describe("SkillStorageLocationSettings", () => {
  beforeEach(() => {
    migrateStorage.mockReset();
    migrateStorage.mockResolvedValue({
      migratedCount: 0,
      skippedCount: 0,
      errors: [],
    });
  });

  it("emits llm_usage_bar when selecting current app storage", async () => {
    const onMigrated = vi.fn();
    render(
      <SkillStorageLocationSettings
        value="unified"
        installedCount={0}
        onMigrated={onMigrated}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "settings.skillStorage.llmUsageBar",
      }),
    );

    await waitFor(() =>
      expect(migrateStorage).toHaveBeenCalledWith("llm_usage_bar"),
    );
    expect(onMigrated).toHaveBeenCalledWith("llm_usage_bar");
  });

  it("keeps llm_usage_bar through the installed-skill confirmation", async () => {
    const onMigrated = vi.fn();
    render(
      <SkillStorageLocationSettings
        value="unified"
        installedCount={2}
        onMigrated={onMigrated}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "settings.skillStorage.llmUsageBar",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "common.confirm" }));

    await waitFor(() =>
      expect(migrateStorage).toHaveBeenCalledWith("llm_usage_bar"),
    );
    expect(onMigrated).toHaveBeenCalledWith("llm_usage_bar");
  });
});
