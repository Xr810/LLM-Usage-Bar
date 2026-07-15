import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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
  });
});
