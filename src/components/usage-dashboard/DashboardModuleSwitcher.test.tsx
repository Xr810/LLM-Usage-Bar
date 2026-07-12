import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { DashboardModuleView } from "@/types/usageDashboard";
import { DashboardModuleSwitcher } from "./DashboardModuleSwitcher";

function dashboardModule(
  id: string,
  name: string,
  sortOrder: number,
  visible = true,
): DashboardModuleView {
  return {
    id,
    name,
    kind: id === "api-id" ? "api" : "subscription",
    sortOrder,
    visible,
    isSystem: id === "api-id",
    providerCount: 0,
  };
}

describe("DashboardModuleSwitcher", () => {
  it("renders any number of visible modules in backend order with tab semantics", () => {
    render(
      <DashboardModuleSwitcher
        modules={[
          dashboardModule("fifth-id", "Fifth", 5),
          dashboardModule("second-id", "Second", 2),
          dashboardModule("hidden-id", "Hidden", 0, false),
          dashboardModule("first-id", "First", 1),
          dashboardModule("api-id", "Fourth", 4),
          dashboardModule("third-id", "Third", 3),
        ]}
        selectedModuleId="first-id"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "First",
      "Second",
      "Third",
      "Fourth",
      "Fifth",
    ]);
    expect(screen.queryByRole("tab", { name: "Hidden" })).toBeNull();
    expect(screen.getByRole("tab", { name: "First" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tablist")).toHaveAccessibleName(
      "Dashboard modules",
    );
  });

  it("activates the next stable ID with ArrowRight and supports direct selection", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    render(
      <DashboardModuleSwitcher
        modules={[
          dashboardModule("first-id", "First", 1),
          dashboardModule("second-id", "Second", 2),
          dashboardModule("third-id", "Third", 3),
        ]}
        selectedModuleId="first-id"
        onSelect={onSelect}
      />,
    );

    await user.click(screen.getByRole("tab", { name: "First" }));
    await user.keyboard("{ArrowRight}");
    expect(onSelect).toHaveBeenCalledWith("second-id");

    await user.click(screen.getByRole("tab", { name: "Third" }));
    expect(onSelect).toHaveBeenCalledWith("third-id");
  });

  it("provides a More menu when many modules overflow", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    render(
      <DashboardModuleSwitcher
        modules={Array.from({ length: 6 }, (_, index) =>
          dashboardModule(
            `module-${index + 1}`,
            `Module ${index + 1}`,
            index + 1,
          ),
        )}
        selectedModuleId="module-1"
        onSelect={onSelect}
      />,
    );

    await user.click(screen.getByRole("button", { name: "More modules" }));
    await user.click(screen.getByRole("menuitem", { name: "Module 6" }));
    expect(onSelect).toHaveBeenCalledWith("module-6");
  });
});
