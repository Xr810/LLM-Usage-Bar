import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DashboardModulesSettings } from "./DashboardModulesSettings";

const mocks = vi.hoisted(() => ({
  modules: [] as Array<Record<string, unknown>>,
  save: vi.fn(),
  reorder: vi.fn(),
  setVisibility: vi.fn(),
  deleteModule: vi.fn(),
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useDashboardModules: () => ({
    data: mocks.modules,
    isLoading: false,
    error: null,
  }),
  useSaveDashboardModule: () => ({
    mutateAsync: mocks.save,
    isPending: false,
  }),
  useReorderDashboardModules: () => ({
    mutateAsync: mocks.reorder,
    isPending: false,
  }),
  useSetDashboardModuleVisibility: () => ({
    mutateAsync: mocks.setVisibility,
    isPending: false,
  }),
  useDeleteDashboardModule: () => ({
    mutateAsync: mocks.deleteModule,
    isPending: false,
  }),
}));

vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({
    isOpen,
    title,
    onConfirm,
  }: {
    isOpen: boolean;
    title: string;
    onConfirm: (checked: boolean) => void;
  }) =>
    isOpen ? (
      <button type="button" onClick={() => onConfirm(false)}>
        Confirm {title}
      </button>
    ) : null,
}));

const moduleFixtures = [
  {
    id: "api",
    name: "API",
    kind: "api",
    sortOrder: 3,
    visible: true,
    isSystem: true,
    providerCount: 2,
  },
  {
    id: "personal",
    name: "Personal",
    kind: "subscription",
    sortOrder: 1,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
  {
    id: "work",
    name: "Work",
    kind: "subscription",
    sortOrder: 2,
    visible: true,
    isSystem: false,
    providerCount: 0,
  },
];

describe("DashboardModulesSettings", () => {
  beforeEach(() => {
    mocks.modules = moduleFixtures.map((module) => ({ ...module }));
    mocks.save.mockReset().mockResolvedValue(undefined);
    mocks.reorder.mockReset().mockResolvedValue(undefined);
    mocks.setVisibility.mockReset().mockResolvedValue(undefined);
    mocks.deleteModule.mockReset().mockResolvedValue(undefined);
  });

  it("orders modules and shows type, visibility, and Provider counts", () => {
    render(<DashboardModulesSettings />);

    expect(
      screen.getByText(
        "Reordering changes dashboard navigation only. Hiding removes a module from navigation and may move the dashboard to another visible module. Neither action changes Provider identity or historical usage.",
      ),
    ).toBeInTheDocument();
    const rows = screen.getAllByTestId(/^dashboard-module-/);
    expect(rows.map((row) => row.dataset.testid)).toEqual([
      "dashboard-module-personal",
      "dashboard-module-work",
      "dashboard-module-api",
    ]);
    expect(within(rows[0]).getByText("1 Provider")).toBeInTheDocument();
    expect(within(rows[1]).getByText("0 Providers")).toBeInTheDocument();
    expect(within(rows[2]).getByText("API module")).toBeInTheDocument();
    expect(
      within(rows[2]).queryByRole("button", { name: "Delete API" }),
    ).toBeNull();
    expect(within(rows[2]).queryByRole("combobox")).toBeNull();
  });

  it("creates, renames, reorders, and hides modules with stable IDs", async () => {
    render(<DashboardModulesSettings />);

    fireEvent.change(screen.getByLabelText("New module name"), {
      target: { value: "Gemini" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create module" }));
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        id: null,
        name: "Gemini",
        kind: "subscription",
        sortOrder: 4,
        visible: true,
      }),
    );

    fireEvent.change(screen.getByLabelText("Name for Work"), {
      target: { value: "Team" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save Work" }));
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        id: "work",
        name: "Team",
        kind: "subscription",
        sortOrder: 2,
        visible: true,
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Move Work up" }));
    await waitFor(() =>
      expect(mocks.reorder).toHaveBeenCalledWith(["work", "personal", "api"]),
    );

    fireEvent.click(screen.getByRole("button", { name: "Hide Work" }));
    await waitFor(() =>
      expect(mocks.setVisibility).toHaveBeenCalledWith({
        moduleId: "work",
        visible: false,
      }),
    );
  });

  it("blocks populated and system deletion, then confirms empty deletion", async () => {
    render(<DashboardModulesSettings />);

    const populated = screen.getByTestId("dashboard-module-personal");
    expect(
      within(populated).getByText(
        "Move or disable its Providers before deleting.",
      ),
    ).toBeInTheDocument();
    expect(
      within(populated).getByRole("button", { name: "Delete Personal" }),
    ).toBeDisabled();
    expect(
      within(screen.getByTestId("dashboard-module-api")).queryByRole("button", {
        name: "Delete API",
      }),
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Delete Work" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Confirm Delete Work" }),
    );
    await waitFor(() =>
      expect(mocks.deleteModule).toHaveBeenCalledWith("work"),
    );
  });

  it("surfaces an authoritative backend rejection", async () => {
    mocks.deleteModule.mockRejectedValueOnce(new Error("module is not empty"));
    render(<DashboardModulesSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Delete Work" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Confirm Delete Work" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "module is not empty",
    );
  });
});
