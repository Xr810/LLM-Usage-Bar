import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

const mocks = vi.hoisted(() => ({
  saveProvider: vi.fn(),
  setEnabled: vi.fn(),
  saveModule: vi.fn(),
  quickCreated: vi.fn(),
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageProviders: () => ({
    data: [
      {
        id: "subscription",
        name: "Official Subscription",
        billingKind: "subscription",
        productGroupId: "codex",
        tokenSources: ["session_log"],
        sessionSourceBindings: ["codex"],
        quotaSource: "codex",
        quotaIntervalSeconds: 300,
        routeAppType: null,
        enabled: true,
        needsReview: false,
        createdAt: 1,
        updatedAt: 1,
        routeBaseUrl: null,
        hasRouteCredentials: false,
        dashboardModuleId: "personal",
      },
      {
        id: "metered",
        name: "Metered API",
        billingKind: "metered",
        productGroupId: "api",
        tokenSources: ["proxy"],
        sessionSourceBindings: [],
        quotaSource: null,
        quotaIntervalSeconds: null,
        routeAppType: "codex",
        enabled: false,
        needsReview: false,
        createdAt: 1,
        updatedAt: 1,
        routeBaseUrl: "https://example.com",
        hasRouteCredentials: true,
        dashboardModuleId: null,
      },
    ],
    isLoading: false,
    error: null,
  }),
  useDashboardModules: () => ({
    data: [
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
        id: "api",
        name: "API",
        kind: "api",
        sortOrder: 2,
        visible: true,
        isSystem: true,
        providerCount: 1,
      },
    ],
    isLoading: false,
    error: null,
  }),
  useSaveUsageProvider: () => ({
    mutateAsync: mocks.saveProvider,
    isPending: false,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: mocks.setEnabled,
    isPending: false,
  }),
  useSaveDashboardModule: () => ({
    mutateAsync: mocks.saveModule,
    isPending: false,
  }),
}));

vi.mock("@/components/usage-dashboard/UsageProviderDialog", () => ({
  UsageProviderDialog: ({
    open,
    provider,
    dashboardModules,
    onCreateModule,
    onSave,
  }: {
    open: boolean;
    provider?: { id: string } | null;
    dashboardModules: Array<{ id: string }>;
    onCreateModule: (name: string) => Promise<{ id: string }>;
    onSave: (input: Record<string, unknown>) => Promise<unknown>;
  }) =>
    open ? (
      <div data-testid="provider-dialog">
        <span>Editing {provider?.id ?? "new"}</span>
        <span>
          Modules {dashboardModules.map((module) => module.id).join(",")}
        </span>
        <button
          type="button"
          onClick={() =>
            void onCreateModule("Quick plan").then(mocks.quickCreated)
          }
        >
          Quick create
        </button>
        <button
          type="button"
          onClick={() =>
            void onSave({ id: provider?.id ?? "new-provider", enabled: true })
          }
        >
          Save mocked Provider
        </button>
      </div>
    ) : null,
}));

describe("UsageProvidersSettings", () => {
  beforeEach(() => {
    mocks.saveProvider.mockReset().mockResolvedValue(undefined);
    mocks.setEnabled.mockReset().mockResolvedValue(undefined);
    mocks.saveModule.mockReset().mockResolvedValue({
      id: "quick-plan",
      name: "Quick plan",
      kind: "subscription",
      sortOrder: 3,
      visible: true,
      isSystem: false,
      providerCount: 0,
    });
    mocks.quickCreated.mockReset();
  });

  it("lists Providers and supports edit and enable state changes", async () => {
    render(<UsageProvidersSettings />);

    expect(screen.getByText("Official Subscription")).toBeInTheDocument();
    expect(screen.getByText("Metered API")).toBeInTheDocument();
    expect(screen.queryByTestId("provider-dialog")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: "Edit Official Subscription" }),
    );
    expect(screen.getByText("Editing subscription")).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Disable Official Subscription" }),
    );
    await waitFor(() =>
      expect(mocks.setEnabled).toHaveBeenCalledWith({
        providerId: "subscription",
        enabled: false,
      }),
    );
  });

  it("wires Add Provider, quick module creation, and Provider save", async () => {
    render(<UsageProvidersSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Add Provider" }));
    expect(screen.getByText("Editing new")).toBeInTheDocument();
    expect(screen.getByText("Modules personal")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Quick create" }));
    await waitFor(() =>
      expect(mocks.saveModule).toHaveBeenCalledWith({
        id: null,
        name: "Quick plan",
        kind: "subscription",
        sortOrder: 3,
        visible: true,
      }),
    );
    await waitFor(() =>
      expect(mocks.quickCreated).toHaveBeenCalledWith(
        expect.objectContaining({ id: "quick-plan" }),
      ),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Save mocked Provider" }),
    );
    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith({
        id: "new-provider",
        enabled: true,
      }),
    );
  });

  it("surfaces Provider mutation failures", async () => {
    mocks.setEnabled.mockRejectedValueOnce(new Error("enable rejected"));
    render(<UsageProvidersSettings />);

    fireEvent.click(
      screen.getByRole("button", { name: "Disable Official Subscription" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "enable rejected",
    );
  });
});
