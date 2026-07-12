import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageProviderDialog } from "./UsageProviderDialog";
import type { UsageProviderView } from "@/types/usageDashboard";

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
    dashboardModuleId: null,
  };
}

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
});
