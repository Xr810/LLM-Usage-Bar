import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SystemProviderAgentBindings } from "./SystemProviderAgentBindings";
import type { UsageProviderView } from "@/types/usageDashboard";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  remove: vi.fn(),
  reveal: vi.fn(),
  rotate: vi.fn(),
  copy: vi.fn(),
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useSaveAgentProviderBinding: () => ({
    mutateAsync: mocks.save,
    isPending: false,
  }),
  useDeleteAgentProviderBinding: () => ({
    mutateAsync: mocks.remove,
    isPending: false,
  }),
  useAgentProviderLocalKeyActions: () => ({
    reveal: mocks.reveal,
    rotate: mocks.rotate,
    isPending: false,
  }),
}));
vi.mock("@/lib/clipboard", () => ({
  copyText: (value: string) => mocks.copy(value),
}));

const provider = {
  id: "system-openrouter-api",
  name: "OpenRouter",
  enabled: true,
  systemAuthKind: "provider_api_key",
  compatibleAgentModuleIds: ["opencode", "openclaw", "hermes"],
  bindings: [
    {
      id: "binding-opencode",
      agentModuleId: "opencode",
      providerId: "system-openrouter-api",
      enabled: true,
      effectiveEnabled: true,
      credentialStatus: "configured",
      canClearCredential: true,
      credentialVersion: 1,
      routeProtocol: "opencode",
      localCredentialStatus: "configured",
      providerCredentialStatus: "configured",
      createdAt: 1,
      updatedAt: 1,
    },
  ],
} as UsageProviderView;

describe("SystemProviderAgentBindings", () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset();
    mocks.save.mockResolvedValue(undefined);
    mocks.remove.mockResolvedValue(undefined);
    mocks.copy.mockResolvedValue(undefined);
  });

  it("edits one binding and copies a revealed local key without rendering it", async () => {
    mocks.reveal.mockResolvedValue({
      bindingId: "binding-opencode",
      credentialVersion: 1,
      localKey: "local-copy-secret",
    });
    render(<SystemProviderAgentBindings provider={provider} />);
    expect(screen.getByLabelText("Use OpenCode")).toBeChecked();
    expect(screen.getByLabelText("Use OpenClaw")).not.toBeChecked();
    fireEvent.click(screen.getByLabelText("Use OpenClaw"));
    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        id: null,
        agentModuleId: "openclaw",
        providerId: "system-openrouter-api",
        enabled: true,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Copy OpenCode local key" }),
    );
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenCalledWith("local-copy-secret"),
    );
    expect(screen.queryByText("local-copy-secret")).toBeNull();
    expect(screen.getByLabelText("OpenCode: Effective")).toBeInTheDocument();
  });

  it("uses distinct accessible labels for missing, unavailable, and disconnected bindings", () => {
    render(
      <SystemProviderAgentBindings
        provider={
          {
            ...provider,
            compatibleAgentModuleIds: [
              "opencode",
              "openclaw",
              "hermes",
              "codex",
            ],
            bindings: [
              provider.bindings[0],
              {
                ...provider.bindings[0],
                id: "binding-openclaw",
                agentModuleId: "openclaw",
                effectiveEnabled: false,
                credentialStatus: "missing",
              },
              {
                ...provider.bindings[0],
                id: "binding-hermes",
                agentModuleId: "hermes",
                effectiveEnabled: false,
                credentialStatus: "unavailable",
              },
              {
                ...provider.bindings[0],
                id: "binding-codex",
                agentModuleId: "codex",
                effectiveEnabled: false,
                credentialStatus: "not_required",
              },
            ],
          } as UsageProviderView
        }
      />,
    );

    expect(screen.getByLabelText("OpenCode: Effective")).toBeInTheDocument();
    expect(screen.getByLabelText("OpenClaw: Missing key")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Hermes: Credential unavailable"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Codex: Requested · Disconnected"),
    ).toBeInTheDocument();
  });

  it("confirms rotation, copies once, and clears the returned local key object", async () => {
    const rotated = {
      bindingId: "binding-opencode",
      credentialVersion: 2,
      localKey: "rotated-local-secret",
    };
    mocks.rotate.mockResolvedValue(rotated);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<SystemProviderAgentBindings provider={provider} />);

    fireEvent.click(
      screen.getByRole("button", { name: "Rotate OpenCode local key" }),
    );
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenCalledWith("rotated-local-secret"),
    );
    expect(mocks.rotate).toHaveBeenCalledWith("binding-opencode", 1);
    expect(rotated.localKey).toBe("");
    expect(screen.queryByText("rotated-local-secret")).toBeNull();
    confirm.mockRestore();
  });
});
