import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SystemProviderApiKeyDialog } from "./SystemProviderApiKeyDialog";
import type { UsageProviderView } from "@/types/usageDashboard";

const provider = {
  id: "system-openrouter-api",
  name: "OpenRouter",
  upstreamCredentialStatus: "missing",
  upstreamCredentialVersion: 0,
} as UsageProviderView;

describe("SystemProviderApiKeyDialog", () => {
  it("starts empty and clears the upstream key after a rejected submission", async () => {
    const submit = vi.fn().mockRejectedValue(new Error("save failed"));
    const { rerender } = render(
      <SystemProviderApiKeyDialog
        open
        provider={provider}
        onOpenChange={vi.fn()}
        onSubmit={submit}
      />,
    );
    const input = screen.getByLabelText("API key") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.value).toBe("");
    fireEvent.change(input, { target: { value: "upstream-dialog-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Save API key" }));
    await waitFor(() =>
      expect(submit).toHaveBeenCalledWith("upstream-dialog-secret"),
    );
    expect(input.value).toBe("");
    expect(screen.queryByText("upstream-dialog-secret")).toBeNull();

    rerender(
      <SystemProviderApiKeyDialog
        open={false}
        provider={provider}
        onOpenChange={vi.fn()}
        onSubmit={submit}
      />,
    );
    rerender(
      <SystemProviderApiKeyDialog
        open
        provider={provider}
        onOpenChange={vi.fn()}
        onSubmit={submit}
      />,
    );
    expect((screen.getByLabelText("API key") as HTMLInputElement).value).toBe(
      "",
    );
  });
});
