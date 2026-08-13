import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageProviderDialog } from "./UsageProviderDialog";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import type {
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";

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
        canClearCredential: true,
        credentialVersion: 4,
        routeProtocol: "codex",
        localCredentialStatus: "not_required",
        providerCredentialStatus: "not_required",
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    quotaSource: null,
    quotaIntervalSeconds: null,
    dailyBudgetUsd: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: "https://upstream.example",
    hasRouteCredentials: true,
    systemPresetKey: null,
    systemAuthKind: null,
    canonicalEndpoint: null,
    compatibleAgentModuleIds: [],
    apiKeys: [],
    supportsKeyUsage: false,
    keyUsageTotal: null,
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
    expect(
      screen.getByLabelText("Idle refresh interval (seconds)"),
    ).toHaveValue(300);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("bindings");
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("dashboardModuleId");
  });

  it("never exposes or serializes protected quota configuration", async () => {
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
        })}
      />,
    );

    expect(screen.queryByLabelText(/quota credential json/i)).toBeNull();
    expect(screen.queryByPlaceholderText(/apiKey/i)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledOnce());
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("quotaConfig");
    expect(onSave.mock.calls[0][0]).not.toHaveProperty("dailyBudgetUsd");
    expect(JSON.stringify(onSave.mock.calls[0][0])).not.toContain(
      "quotaConfig",
    );
  });

  it("rejects daily budgets sent through the generic Provider-save boundary", async () => {
    const input = {
      id: "wrong-budget-channel",
      name: "Wrong Budget Channel",
      billingKind: "metered",
      productGroupId: "api",
      tokenSources: ["proxy"],
      sessionSourceBindings: [],
      quotaSource: null,
      quotaIntervalSeconds: null,
      routeAppType: "codex",
      routeConfig: null,
      dailyBudgetUsd: "99",
      enabled: true,
    } as unknown as UsageProviderInput;

    await expect(usageDashboardApi.saveProvider(input)).rejects.toThrow(
      "renderer_daily_budget_forbidden",
    );
  });

  it("has an MSW command boundary that rejects legacy quotaConfig payloads without echoing their value", async () => {
    const sentinel = "legacy-renderer-quota-secret";
    const input = {
      id: "legacy-provider",
      name: "Legacy Provider",
      billingKind: "subscription",
      productGroupId: "legacy",
      tokenSources: ["session_log"],
      sessionSourceBindings: ["codex"],
      quotaSource: "codex",
      quotaIntervalSeconds: 300,
      routeAppType: null,
      routeConfig: null,
      quotaConfig: { apiKey: sentinel },
      enabled: true,
    } as unknown as UsageProviderInput;

    let error: unknown;
    try {
      await usageDashboardApi.saveProvider(input);
    } catch (cause) {
      error = cause;
    }

    expect(error).toBeInstanceOf(Error);
    expect(String(error)).toContain("renderer_quota_config_forbidden");
    expect(String(error)).not.toContain(sentinel);
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

    fireEvent.change(screen.getByLabelText("Idle refresh interval (seconds)"), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Refresh interval must be 0 or at least 60 seconds.",
    );
    expect(onSave).not.toHaveBeenCalled();
  });
});
