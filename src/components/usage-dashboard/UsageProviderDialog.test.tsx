import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageProviderDialog } from "./UsageProviderDialog";
import type { UsageProviderView } from "@/types/usageDashboard";

function provider(
  overrides: Partial<UsageProviderView> = {},
): UsageProviderView {
  return {
    id: "provider-a",
    name: "Provider A",
    billingKind: "metered",
    productGroupId: "api",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    bindings: [
      {
        id: "binding-a",
        agentModuleId: "codex",
        providerId: "provider-a",
        enabled: true,
        effectiveEnabled: true,
        credentialStatus: "configured",
        credentialVersion: 4,
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: "https://upstream.example",
    hasRouteCredentials: true,
    ...overrides,
  };
}

describe("UsageProviderDialog", () => {
  it("creates metered Provider metadata without module or route-key fields", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<UsageProviderDialog open onOpenChange={vi.fn()} onSave={onSave} />);

    expect(screen.queryByText("Usage module")).toBeNull();
    expect(screen.queryByText("New module name")).toBeNull();
    expect(screen.queryByLabelText(/API key/i)).toBeNull();

    fireEvent.change(screen.getByLabelText("ID"), {
      target: { value: "new-provider" },
    });
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "New Provider" },
    });
    fireEvent.change(screen.getByLabelText("Product group"), {
      target: { value: "new-api" },
    });
    fireEvent.change(screen.getByLabelText("Route protocol"), {
      target: { value: "gemini" },
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://new.example" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    const input = onSave.mock.calls[0][0];
    expect(input).toMatchObject({
      id: "new-provider",
      name: "New Provider",
      productGroupId: "new-api",
      billingKind: "metered",
      routeAppType: "gemini",
      routeConfig: { baseUrl: "https://new.example" },
    });
    expect(input).not.toHaveProperty("dashboardModuleId");
    expect(input).not.toHaveProperty("bindings");
    expect(JSON.stringify(input)).not.toContain("apiKey");
  });

  it("edits a subscription Provider without displaying Agent membership", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        provider={provider({
          billingKind: "subscription",
          quotaSource: "codex",
          quotaIntervalSeconds: 300,
          routeAppType: null,
          routeBaseUrl: null,
          hasRouteCredentials: false,
        })}
      />,
    );

    expect(screen.queryByText("Usage module")).toBeNull();
    expect(screen.queryByText("codex", { selector: "option" })).toBeNull();
    expect(screen.getByLabelText("Refresh interval (seconds)")).toHaveValue(
      300,
    );

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("bindings");
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("dashboardModuleId");
  });

  it("does not expose or resubmit a legacy route API key", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        provider={provider()}
      />,
    );

    expect(screen.queryByLabelText(/API key/i)).toBeNull();
    expect(screen.queryByDisplayValue(/sk-/i)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(JSON.stringify(onSave.mock.calls[0][0])).not.toContain("apiKey");
  });

  it("keeps session source ownership explicit", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        provider={provider({ tokenSources: ["proxy", "session_log"] })}
      />,
    );

    fireEvent.click(screen.getByLabelText("Codex session logs"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0].sessionSourceBindings).toEqual(["codex"]);
  });

  it("rejects invalid subscription refresh intervals", async () => {
    const onSave = vi.fn();
    render(
      <UsageProviderDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        provider={provider({
          billingKind: "subscription",
          quotaIntervalSeconds: 300,
          routeAppType: null,
        })}
      />,
    );

    fireEvent.change(screen.getByLabelText("Refresh interval (seconds)"), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Refresh interval must be 0 or at least 60 seconds.",
    );
    expect(onSave).not.toHaveBeenCalled();
  });
});
