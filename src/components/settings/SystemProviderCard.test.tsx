import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { SystemProviderCard } from "./SystemProviderCard";
import type { UsageProviderView } from "@/types/usageDashboard";

const mocks = vi.hoisted(() => ({
  setApiKey: vi.fn(),
  replaceApiKey: vi.fn(),
  testConnection: vi.fn(),
}));

vi.mock("./ProviderDailyBudgetField", () => ({
  ProviderDailyBudgetField: ({
    providerId,
    targeted,
  }: {
    providerId: string;
    targeted?: boolean;
  }) => (
    <div data-testid={`budget-${providerId}`} data-targeted={targeted}>
      Daily budget
    </div>
  ),
}));
vi.mock("./ProviderModelPricingSection", () => ({
  ProviderModelPricingSection: ({ providerId }: { providerId: string }) => (
    <div data-testid={`pricing-${providerId}`}>Model prices</div>
  ),
}));
vi.mock("./ClaudeCliAuthSection", () => ({
  ClaudeCliAuthSection: () => <div>Claude auth</div>,
}));
vi.mock("@/components/providers/forms/CodexOAuthSection", () => ({
  CodexOAuthSection: () => <div>ChatGPT auth</div>,
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useSystemProviderCredentialActions: () => ({
    isPending: false,
    setApiKey: mocks.setApiKey,
    replaceApiKey: mocks.replaceApiKey,
    testConnection: mocks.testConnection,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

beforeEach(() => {
  mocks.setApiKey.mockReset().mockResolvedValue({
    upstreamCredentialVersion: 1,
  });
  mocks.replaceApiKey.mockReset().mockResolvedValue({
    upstreamCredentialVersion: 2,
  });
  mocks.testConnection.mockReset().mockResolvedValue({
    status: "success",
  });
});

it("shows one inline API key field and verifies with the saved credential version", async () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={
        {
          id: "system-openai-api",
          name: "OpenAI API",
          billingKind: "metered",
          dailyBudgetUsd: null,
          systemAuthKind: "provider_api_key",
          canonicalEndpoint: "https://api.openai.com/v1",
          upstreamCredentialStatus: "missing",
          upstreamCredentialVersion: 0,
          canClearUpstreamCredential: false,
          bindings: [],
          enabled: true,
        } as unknown as UsageProviderView
      }
    />,
  );
  expect(screen.getByText("OpenAI API")).toBeInTheDocument();
  expect(screen.queryByText("https://api.openai.com/v1")).toBeNull();
  expect(screen.queryByText("Endpoint locked")).toBeNull();
  const input = screen.getByLabelText("API key") as HTMLInputElement;
  expect(input).toHaveAttribute("placeholder", "Enter API key");
  expect(input.type).toBe("password");
  const verifyButton = screen.getByRole("button", { name: "Verify" });
  expect(verifyButton).toBeDisabled();
  fireEvent.change(input, { target: { value: "openai-test-key" } });
  fireEvent.click(verifyButton);

  await waitFor(() =>
    expect(mocks.setApiKey).toHaveBeenCalledWith(
      "system-openai-api",
      0,
      "openai-test-key",
    ),
  );
  expect(mocks.testConnection).toHaveBeenCalledWith("system-openai-api", 1);
  expect(input.value).toBe("");
  expect(screen.getByText("Connection succeeded")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /edit/i })).toBeNull();
  expect(screen.queryByRole("button", { name: /delete/i })).toBeNull();
  expect(screen.queryByRole("button", { name: "Set API key" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Clear API key" })).toBeNull();
});

it("places a targeted budget editor after authentication for metered Providers", () => {
  const onTargetHandled = vi.fn();
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={
        {
          id: "system-openai-api",
          name: "OpenAI API",
          billingKind: "metered",
          dailyBudgetUsd: "10",
          systemAuthKind: "provider_api_key",
          canonicalEndpoint: "https://api.openai.com/v1",
          upstreamCredentialStatus: "missing",
          upstreamCredentialVersion: 0,
          canClearUpstreamCredential: false,
          bindings: [],
          enabled: true,
        } as unknown as UsageProviderView
      }
      targetProviderId="system-openai-api"
      onTargetHandled={onTargetHandled}
    />,
  );

  const cardText = document.body.textContent ?? "";
  expect(screen.getByTestId("budget-system-openai-api")).toHaveAttribute(
    "data-targeted",
    "true",
  );
  expect(cardText.indexOf("Verify")).toBeLessThan(
    cardText.indexOf("Daily budget"),
  );
  expect(cardText).not.toContain("Agent bindings");
});

it("does not render a budget editor for subscription Providers", () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={
        {
          id: "system-chatgpt-subscription",
          name: "ChatGPT",
          billingKind: "subscription",
          dailyBudgetUsd: null,
          systemAuthKind: "codex_oauth",
          canonicalEndpoint: null,
          upstreamCredentialStatus: "not_required",
          upstreamCredentialVersion: 0,
          canClearUpstreamCredential: false,
          bindings: [],
          enabled: true,
        } as unknown as UsageProviderView
      }
      targetProviderId="system-chatgpt-subscription"
    />,
  );

  expect(screen.queryByText("Daily budget")).toBeNull();
});

it("saves NVIDIA's API key without calling its unsupported connection test", async () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={
        {
          id: "system-nvidia-nim-api",
          name: "NVIDIA NIM API",
          billingKind: "metered",
          dailyBudgetUsd: null,
          systemPresetKey: "nvidia-nim-api",
          systemAuthKind: "provider_api_key",
          canonicalEndpoint: "https://integrate.api.nvidia.com/v1",
          upstreamCredentialStatus: "configured",
          upstreamCredentialVersion: 1,
          canClearUpstreamCredential: true,
          lastConnectionTestStatus: null,
          bindings: [],
          enabled: true,
        } as unknown as UsageProviderView
      }
    />,
  );

  const input = screen.getByLabelText("API key");
  fireEvent.change(input, { target: { value: "nvidia-test-key" } });
  fireEvent.click(screen.getByRole("button", { name: "Verify" }));

  await waitFor(() =>
    expect(mocks.replaceApiKey).toHaveBeenCalledWith(
      "system-nvidia-nim-api",
      1,
      "nvidia-test-key",
    ),
  );
  expect(mocks.testConnection).not.toHaveBeenCalled();
  expect(screen.queryByText("https://integrate.api.nvidia.com/v1")).toBeNull();
});
