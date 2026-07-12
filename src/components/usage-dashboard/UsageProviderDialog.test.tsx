import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageProviderDialog } from "./UsageProviderDialog";
import type {
  DashboardModuleView,
  UsageProviderView,
} from "@/types/usageDashboard";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key,
  }),
}));

function existingProvider(): UsageProviderView {
  return {
    id: "subscription",
    name: "Subscription",
    billingKind: "subscription",
    productGroupId: "claude",
    tokenSources: ["session_log"],
    sessionSourceBindings: ["codex"],
    quotaSource: "claude",
    quotaIntervalSeconds: 300,
    routeAppType: null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: null,
    hasRouteCredentials: false,
    dashboardModuleId: "module-a",
  };
}

const modules: DashboardModuleView[] = [
  {
    id: "module-a",
    name: "Primary plan",
    kind: "subscription",
    sortOrder: 1,
    visible: true,
    isSystem: false,
    providerCount: 1,
  },
  {
    id: "module-b",
    name: "Secondary plan",
    kind: "subscription",
    sortOrder: 2,
    visible: true,
    isSystem: false,
    providerCount: 0,
  },
  {
    id: "module-api",
    name: "API",
    kind: "api",
    sortOrder: 3,
    visible: true,
    isSystem: true,
    providerCount: 1,
  },
];

describe("UsageProviderDialog session source ownership", () => {
  it("lets a new provider explicitly claim Claude and Codex session logs", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<UsageProviderDialog open onOpenChange={vi.fn()} onSave={onSave} />);

    fireEvent.click(screen.getByLabelText("Session log"));
    fireEvent.click(screen.getByLabelText("Claude session logs"));
    fireEvent.click(screen.getByLabelText("Codex session logs"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].sessionSourceBindings).toEqual([
      "claude",
      "codex",
    ]);
  });

  it("round-trips and explicitly clears an existing provider binding", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        provider={existingProvider()}
        dashboardModules={modules}
        onSave={onSave}
      />,
    );

    const codex = screen.getByLabelText("Codex session logs");
    expect(codex).toBeChecked();
    fireEvent.click(codex);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].sessionSourceBindings).toEqual([]);
  });

  it("requires and saves a stable subscription module ID", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        dashboardModules={modules}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Billing"), {
      target: { value: "subscription" },
    });
    expect(screen.queryByRole("option", { name: "API" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByRole("alert", {
        name: "Choose a subscription module.",
      }),
    ).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("Usage module"), {
      target: { value: "module-b" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].dashboardModuleId).toBe("module-b");
  });

  it("moves an existing subscription Provider without changing its ID", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const sharedModules = modules.map((module) =>
      module.id === "module-a" ? { ...module, providerCount: 2 } : module,
    );
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        provider={existingProvider()}
        dashboardModules={sharedModules}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Usage module"), {
      target: { value: "module-b" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      id: "subscription",
      dashboardModuleId: "module-b",
    });
  });

  it("confirms before moving the last Provider out of a module", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onOpenChange = vi.fn();
    render(
      <UsageProviderDialog
        open
        onOpenChange={onOpenChange}
        provider={existingProvider()}
        dashboardModules={modules}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Usage module"), {
      target: { value: "module-b" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByRole("heading", { name: "Leave Primary plan empty?" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Moving Subscription will leave Primary plan without Providers. Historical usage is unchanged.",
      ),
    ).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalledWith(false);

    fireEvent.click(screen.getByRole("button", { name: "Move Provider" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      id: "subscription",
      dashboardModuleId: "module-b",
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("keeps the expanded Provider form body scrollable", () => {
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        provider={existingProvider()}
        dashboardModules={modules}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByTestId("provider-form-scroll-region")).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-y-auto",
    );
  });

  it("quick-creates and selects a subscription module", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onCreateModule = vi.fn().mockResolvedValue({
      ...modules[1],
      id: "module-new",
      name: "New plan",
    });
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        dashboardModules={modules}
        onCreateModule={onCreateModule}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Billing"), {
      target: { value: "subscription" },
    });
    fireEvent.change(screen.getByLabelText("New module name"), {
      target: { value: "New plan" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create module" }));
    await waitFor(() =>
      expect(onCreateModule).toHaveBeenCalledWith("New plan"),
    );
    expect(screen.getByLabelText("Usage module")).toHaveValue("module-new");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].dashboardModuleId).toBe("module-new");
  });

  it("allows a disabled subscription Provider to remain unassigned", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        dashboardModules={modules}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Billing"), {
      target: { value: "subscription" },
    });
    fireEvent.click(screen.getByLabelText("Enabled"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      billingKind: "subscription",
      dashboardModuleId: null,
      enabled: false,
    });
  });

  it("hides module selection and saves null for metered billing", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const sharedModules = modules.map((module) =>
      module.id === "module-a" ? { ...module, providerCount: 2 } : module,
    );
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        provider={existingProvider()}
        dashboardModules={sharedModules}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByLabelText("Billing"), {
      target: { value: "metered" },
    });
    expect(screen.queryByLabelText("Usage module")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].dashboardModuleId).toBeNull();
  });
});
