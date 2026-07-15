import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AgentProviderBindingView } from "@/types/usageDashboard";
import { ProtectedBindingKeyDialog } from "./ProtectedBindingKeyDialog";

const binding: AgentProviderBindingView = {
  id: "binding-a",
  agentModuleId: "codex",
  providerId: "provider-a",
  enabled: true,
  effectiveEnabled: true,
  credentialStatus: "missing",
  canClearCredential: false,
  credentialVersion: 2,
  routeProtocol: "codex",
  localCredentialStatus: "not_required",
  providerCredentialStatus: "not_required",
  createdAt: 1,
  updatedAt: 1,
};

describe("ProtectedBindingKeyDialog", () => {
  it("uses an empty password-only input and clears it on failure and reopen", async () => {
    const setApiKey = vi.fn().mockRejectedValue(new Error("key rejected"));
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <ProtectedBindingKeyDialog
        open
        mode="set"
        binding={binding}
        onOpenChange={onOpenChange}
        onSetApiKey={setApiKey}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={vi.fn()}
      />,
    );

    const input = screen.getByLabelText("API key");
    expect(input).toHaveAttribute("type", "password");
    expect(input).toHaveValue("");
    fireEvent.change(input, { target: { value: "transient-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Set API key" }));

    await waitFor(() =>
      expect(setApiKey).toHaveBeenCalledWith(
        "binding-a",
        2,
        "transient-secret",
      ),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("key rejected");
    expect(screen.getByLabelText("API key")).toHaveValue("");

    rerender(
      <ProtectedBindingKeyDialog
        open={false}
        mode="set"
        binding={binding}
        onOpenChange={onOpenChange}
        onSetApiKey={setApiKey}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={vi.fn()}
      />,
    );
    rerender(
      <ProtectedBindingKeyDialog
        open
        mode="replace"
        binding={{ ...binding, credentialStatus: "configured" }}
        onOpenChange={onOpenChange}
        onSetApiKey={setApiKey}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={vi.fn()}
      />,
    );
    expect(screen.getByLabelText("API key")).toHaveValue("");
  });

  it("clears without rendering any key input", async () => {
    const clearApiKey = vi.fn().mockResolvedValue(undefined);
    render(
      <ProtectedBindingKeyDialog
        open
        mode="clear"
        binding={{ ...binding, credentialStatus: "configured" }}
        onOpenChange={vi.fn()}
        onSetApiKey={vi.fn()}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={clearApiKey}
      />,
    );

    expect(screen.queryByLabelText("API key")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Clear API key" }));
    await waitFor(() =>
      expect(clearApiKey).toHaveBeenCalledWith("binding-a", 2),
    );
  });

  it("freezes the expected credential version while the dialog is open", async () => {
    const setApiKey = vi.fn().mockRejectedValue(new Error("version conflict"));
    const { rerender } = render(
      <ProtectedBindingKeyDialog
        open
        mode="set"
        binding={binding}
        onOpenChange={vi.fn()}
        onSetApiKey={setApiKey}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("API key"), {
      target: { value: "stale-operation-key" },
    });
    rerender(
      <ProtectedBindingKeyDialog
        open
        mode="set"
        binding={{ ...binding, credentialVersion: 3 }}
        onOpenChange={vi.fn()}
        onSetApiKey={setApiKey}
        onReplaceApiKey={vi.fn()}
        onClearApiKey={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Set API key" }));

    await waitFor(() =>
      expect(setApiKey).toHaveBeenCalledWith(
        "binding-a",
        2,
        "stale-operation-key",
      ),
    );
  });
});
