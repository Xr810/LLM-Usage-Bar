import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

const mocks = vi.hoisted(() => ({
  saveProvider: vi.fn(),
  deleteProvider: vi.fn(),
  setEnabled: vi.fn(),
  bindingMutation: vi.fn(),
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
        bindings: [
          {
            id: "binding-codex",
            agentModuleId: "codex",
            providerId: "subscription",
            enabled: true,
            effectiveEnabled: true,
            credentialStatus: "not_required",
            canClearCredential: false,
            credentialVersion: 2,
            createdAt: 1,
            updatedAt: 1,
          },
        ],
        quotaSource: "codex",
        quotaIntervalSeconds: 300,
        dailyBudgetUsd: null,
        routeAppType: null,
        enabled: true,
        needsReview: false,
        createdAt: 1,
        updatedAt: 1,
        routeBaseUrl: null,
        hasRouteCredentials: false,
      },
      {
        id: "metered",
        name: "Metered API",
        billingKind: "metered",
        productGroupId: "api",
        tokenSources: ["proxy"],
        sessionSourceBindings: [],
        bindings: [],
        quotaSource: null,
        quotaIntervalSeconds: null,
        dailyBudgetUsd: "10",
        routeAppType: "codex",
        enabled: false,
        needsReview: false,
        createdAt: 1,
        updatedAt: 1,
        routeBaseUrl: "https://example.com",
        hasRouteCredentials: false,
      },
    ],
    isLoading: false,
    error: null,
  }),
  useSaveUsageProvider: () => ({
    mutateAsync: mocks.saveProvider,
    isPending: false,
  }),
  useDeleteUsageProvider: () => ({
    mutateAsync: mocks.deleteProvider,
    isPending: false,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: mocks.setEnabled,
    isPending: false,
  }),
  useSaveAgentProviderBinding: () => ({
    mutateAsync: mocks.bindingMutation,
    isPending: false,
  }),
}));

vi.mock("@/lib/query/trayUsage", () => ({
  useApiBudgetConfig: () => ({
    data: { mode: "shared", sharedDailyBudgetUsd: null },
    isLoading: false,
    error: null,
  }),
  useSetApiBudgetConfig: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/components/usage-dashboard/UsageProviderDialog", () => ({
  UsageProviderDialog: ({
    open,
    provider,
    onSave,
  }: {
    open: boolean;
    provider?: { id: string; bindings: Array<{ id: string }> } | null;
    onSave: (input: Record<string, unknown>) => Promise<unknown>;
  }) =>
    open ? (
      <div data-testid="provider-dialog">
        <span>Editing {provider?.id ?? "new"}</span>
        <span>Preserved bindings {provider?.bindings.length ?? 0}</span>
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

vi.mock("./ProviderDailyBudgetField", () => ({
  ProviderDailyBudgetField: () => <div>Daily budget field</div>,
}));

vi.mock("./ProviderModelPricingSection", () => ({
  ProviderModelPricingSection: () => <div>Model prices section</div>,
}));

vi.mock("./OfficialPricingRefreshSection", () => ({
  OfficialPricingRefreshSection: () => <div>Official prices section</div>,
}));

describe("UsageProvidersSettings", () => {
  beforeEach(() => {
    mocks.saveProvider.mockReset().mockResolvedValue(undefined);
    mocks.deleteProvider.mockReset().mockResolvedValue(undefined);
    mocks.setEnabled.mockReset().mockResolvedValue(undefined);
    mocks.bindingMutation.mockReset().mockResolvedValue(undefined);
  });

  it("lists Providers and supports edit and enable state changes", async () => {
    render(<UsageProvidersSettings />);

    expect(screen.getByText("Official Subscription")).toBeInTheDocument();
    expect(screen.getByText("Metered API")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Combined API daily limit (USD)"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Daily budget field")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: "Edit Official Subscription" }),
    );
    expect(screen.getByText("Editing subscription")).toBeInTheDocument();
    expect(screen.getByText("Preserved bindings 1")).toBeInTheDocument();

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

  it("saves Provider metadata without any binding mutation", async () => {
    render(<UsageProvidersSettings />);

    fireEvent.click(screen.getByRole("button", { name: "Add Provider" }));
    expect(screen.getByText("Editing new")).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Save mocked Provider" }),
    );
    await waitFor(() =>
      expect(mocks.saveProvider).toHaveBeenCalledWith({
        id: "new-provider",
        enabled: true,
      }),
    );
    expect(mocks.bindingMutation).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
  });

  it("keeps the custom creation entry and deletes a selected custom Provider after confirmation", async () => {
    render(<UsageProvidersSettings />);

    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete Metered API" }));
    expect(
      screen.getByText(/Are you sure.*Metered API.*cannot be undone/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() =>
      expect(mocks.deleteProvider).toHaveBeenCalledWith("metered"),
    );
    expect(
      screen.getByRole("button", { name: "Add Provider" }),
    ).toBeInTheDocument();
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
