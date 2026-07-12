import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { DashboardModuleView } from "@/types/usageDashboard";
import { useDashboardModuleSelection } from "./useDashboardModuleSelection";

const storageKey = "llm-usage-bar:last-dashboard-module-id";

function dashboardModule(
  id: string,
  sortOrder: number,
  overrides: Partial<DashboardModuleView> = {},
): DashboardModuleView {
  return {
    id,
    name: `Display ${id}`,
    kind: "subscription",
    sortOrder,
    visible: true,
    isSystem: false,
    providerCount: 0,
    ...overrides,
  };
}

describe("useDashboardModuleSelection", () => {
  beforeEach(() => localStorage.clear());

  it("selects the first visible module by sort order and persists changes by stable ID", async () => {
    const modules = [
      dashboardModule("second-id", 2),
      dashboardModule("hidden-id", 0, { visible: false }),
      dashboardModule("first-id", 1),
    ];
    const { result } = renderHook(() => useDashboardModuleSelection(modules));

    expect(result.current.selectedModule?.id).toBe("first-id");
    await waitFor(() =>
      expect(localStorage.getItem(storageKey)).toBe("first-id"),
    );

    act(() => result.current.selectModule("second-id"));
    expect(result.current.selectedModule?.id).toBe("second-id");
    expect(localStorage.getItem(storageKey)).toBe("second-id");
  });

  it("restores a visible persisted stable ID", () => {
    localStorage.setItem(storageKey, "second-id");
    const { result } = renderHook(() =>
      useDashboardModuleSelection([
        dashboardModule("first-id", 1),
        dashboardModule("second-id", 2),
      ]),
    );

    expect(result.current.selectedModule?.id).toBe("second-id");
  });

  it("falls back after the current module is hidden or deleted", async () => {
    localStorage.setItem(storageKey, "second-id");
    const { result, rerender } = renderHook(
      ({ modules }: { modules: DashboardModuleView[] }) =>
        useDashboardModuleSelection(modules),
      {
        initialProps: {
          modules: [
            dashboardModule("first-id", 1),
            dashboardModule("second-id", 2),
          ],
        },
      },
    );
    expect(result.current.selectedModule?.id).toBe("second-id");

    rerender({
      modules: [
        dashboardModule("first-id", 1),
        dashboardModule("second-id", 2, { visible: false }),
      ],
    });

    expect(result.current.selectedModule?.id).toBe("first-id");
    await waitFor(() =>
      expect(localStorage.getItem(storageKey)).toBe("first-id"),
    );
  });

  it("selects the API module when it is the only visible module", () => {
    const api = dashboardModule("api-stable-id", 10, {
      name: "Anything metered",
      kind: "api",
      isSystem: true,
    });
    const { result } = renderHook(() =>
      useDashboardModuleSelection([
        dashboardModule("subscription-id", 1, { visible: false }),
        api,
      ]),
    );

    expect(result.current.selectedModule).toEqual(api);
  });

  it("returns null and clears persistence when no module is visible", async () => {
    localStorage.setItem(storageKey, "hidden-id");
    const { result } = renderHook(() =>
      useDashboardModuleSelection([
        dashboardModule("hidden-id", 1, { visible: false }),
      ]),
    );

    expect(result.current.selectedModule).toBeNull();
    await waitFor(() => expect(localStorage.getItem(storageKey)).toBeNull());
  });
});
