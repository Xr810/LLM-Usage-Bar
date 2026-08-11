import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { SystemProviderCard } from "./SystemProviderCard";
import type {
  ProviderApiKeyView,
  UsageProviderView,
} from "@/types/usageDashboard";

const mocks = vi.hoisted(() => ({
  setApiKey: vi.fn(),
  replaceApiKey: vi.fn(),
  clearApiKey: vi.fn(),
  testConnection: vi.fn(),
  refreshKeyUsage: vi.fn(),
  createKey: vi.fn(),
  renameKey: vi.fn(),
  deleteKey: vi.fn(),
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
    clearApiKey: mocks.clearApiKey,
    testConnection: mocks.testConnection,
    refreshKeyUsage: mocks.refreshKeyUsage,
    createKey: mocks.createKey,
    renameKey: mocks.renameKey,
    deleteKey: mocks.deleteKey,
  }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

function keyView(overrides: Partial<ProviderApiKeyView> = {}) {
  return {
    id: "key-a",
    providerId: "system-openrouter-api",
    label: "OpenClaw",
    credentialStatus: "configured",
    credentialVersion: 2,
    canClearCredential: true,
    lastConnectionTestAt: 1786355315,
    lastConnectionTestStatus: "success",
    lastConnectionTestErrorCode: null,
    sortOrder: 0,
    keyUsage: {
      usageTotalUsd: "32.1",
      usageDailyUsd: "1.2",
      usageWeeklyUsd: "8.4",
      usageMonthlyUsd: "12.34",
      limitUsd: "100",
      limitRemainingUsd: "67.9",
      isFreeTier: false,
      fetchedAt: 1786355315,
      credentialVersion: 2,
      stale: false,
    },
    ...overrides,
  } as ProviderApiKeyView;
}

function provider(overrides: Record<string, unknown> = {}) {
  return {
    id: "system-openrouter-api",
    name: "OpenRouter",
    billingKind: "metered",
    dailyBudgetUsd: null,
    systemPresetKey: "openrouter-api",
    systemAuthKind: "provider_api_key",
    canonicalEndpoint: "https://openrouter.ai/api/v1",
    supportsKeyUsage: true,
    apiKeys: [keyView()],
    keyUsageTotal: null,
    bindings: [],
    enabled: true,
    ...overrides,
  } as unknown as UsageProviderView;
}

beforeEach(() => {
  mocks.setApiKey.mockReset().mockResolvedValue({ credentialVersion: 1 });
  mocks.replaceApiKey.mockReset().mockResolvedValue({ credentialVersion: 3 });
  mocks.clearApiKey.mockReset().mockResolvedValue({ credentialVersion: 3 });
  mocks.testConnection.mockReset().mockResolvedValue({ status: "success" });
  mocks.refreshKeyUsage.mockReset().mockResolvedValue({});
  mocks.createKey.mockReset().mockResolvedValue(keyView({ id: "key-b" }));
  mocks.renameKey.mockReset().mockResolvedValue(keyView());
  mocks.deleteKey.mockReset().mockResolvedValue(undefined);
});

it("lists each key by name with its own spend", () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        apiKeys: [
          keyView(),
          keyView({
            id: "key-b",
            label: "Spare",
            keyUsage: { ...keyView().keyUsage!, usageTotalUsd: "26.8" },
          }),
        ],
      })}
    />,
  );

  expect(screen.getByText("OpenClaw")).toBeInTheDocument();
  expect(screen.getByText("Spare")).toBeInTheDocument();
  // Amounts format in the app language (zh in tests), hence the US$ prefix.
  expect(screen.getByText("US$32.10")).toBeInTheDocument();
  expect(screen.getByText("US$26.80")).toBeInTheDocument();
});

it("shows a Provider total only once a second key contributes to it", () => {
  const total = {
    usageTotalUsd: "58.9",
    usageDailyUsd: "1.2",
    usageWeeklyUsd: "8.4",
    usageMonthlyUsd: "12.34",
    limitUsd: null,
    limitRemainingUsd: null,
    isFreeTier: null,
    fetchedAt: 1786355315,
    credentialVersion: 0,
    stale: false,
  };

  const { rerender } = render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({ keyUsageTotal: total })}
    />,
  );
  // One key: the total would just repeat that key's own figure.
  expect(screen.queryByText(/Total across all/)).toBeNull();

  rerender(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        apiKeys: [keyView(), keyView({ id: "key-b", label: "Spare" })],
        keyUsageTotal: total,
      })}
    />,
  );
  expect(screen.getByText(/Total across all/)).toBeInTheDocument();
  expect(screen.getByText("US$58.90")).toBeInTheDocument();
});

it("acts on the key that was clicked, not the Provider", async () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        apiKeys: [
          keyView(),
          keyView({ id: "key-b", label: "Spare", credentialVersion: 5 }),
        ],
      })}
    />,
  );

  fireEvent.click(
    screen.getAllByRole("button", { name: "Test connection" })[1],
  );

  await waitFor(() =>
    expect(mocks.testConnection).toHaveBeenCalledWith("key-b", 5),
  );
});

it("adds a named key", async () => {
  render(<SystemProviderCard onRemove={vi.fn()} provider={provider()} />);

  fireEvent.click(screen.getByRole("button", { name: /Add key/ }));
  fireEvent.change(screen.getByLabelText("Key name"), {
    target: { value: "Spare" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));

  await waitFor(() =>
    expect(mocks.createKey).toHaveBeenCalledWith(
      "system-openrouter-api",
      "Spare",
    ),
  );
});

it("renames a key inline", async () => {
  render(<SystemProviderCard onRemove={vi.fn()} provider={provider()} />);

  fireEvent.click(screen.getByRole("button", { name: "OpenClaw" }));
  const input = screen.getByLabelText("Key name");
  fireEvent.change(input, { target: { value: "Renamed" } });
  fireEvent.keyDown(input, { key: "Enter" });

  await waitFor(() =>
    expect(mocks.renameKey).toHaveBeenCalledWith("key-a", "Renamed"),
  );
});

it("deletes a key only after the confirmation is accepted", async () => {
  render(<SystemProviderCard onRemove={vi.fn()} provider={provider()} />);

  fireEvent.click(screen.getByRole("button", { name: "Delete OpenClaw" }));
  expect(mocks.deleteKey).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Delete" }));

  await waitFor(() => expect(mocks.deleteKey).toHaveBeenCalledWith("key-a", 2));
});

it("offers an input for a key that has no credential yet", async () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        apiKeys: [
          keyView({
            credentialStatus: "missing",
            credentialVersion: 0,
            canClearCredential: false,
            keyUsage: null,
          }),
        ],
      })}
    />,
  );

  fireEvent.change(screen.getByLabelText("API key"), {
    target: { value: "sk-or-new" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Verify" }));

  await waitFor(() =>
    expect(mocks.setApiKey).toHaveBeenCalledWith("key-a", 0, "sk-or-new"),
  );
  expect(mocks.testConnection).toHaveBeenCalledWith("key-a", 1);
});

it("expands one key to its daily, monthly and remaining figures", () => {
  render(<SystemProviderCard onRemove={vi.fn()} provider={provider()} />);

  expect(screen.queryByText("US$1.20")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Expand OpenClaw" }));

  expect(screen.getByText("US$1.20")).toBeInTheDocument();
  expect(screen.getByText("US$12.34")).toBeInTheDocument();
  expect(screen.getByText("US$67.90 left of US$100.00")).toBeInTheDocument();
});

it("explains a rejected duplicate key rather than blaming the Provider", async () => {
  mocks.replaceApiKey.mockRejectedValue("credential_conflict");
  render(<SystemProviderCard onRemove={vi.fn()} provider={provider()} />);

  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  fireEvent.change(screen.getByLabelText("API key"), {
    target: { value: "sk-or-same" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Verify" }));

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(/already the saved one/);
});

it("offers no connection test for a preset that cannot validate a key", () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        id: "system-nvidia-nim-api",
        name: "NVIDIA NIM API",
        systemPresetKey: "nvidia-nim-api",
        supportsKeyUsage: false,
      })}
    />,
  );

  expect(screen.queryByRole("button", { name: "Test connection" })).toBeNull();
  expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
});

it("does not render a budget editor for subscription Providers", () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({
        id: "system-chatgpt-subscription",
        name: "ChatGPT",
        billingKind: "subscription",
        systemAuthKind: "codex_oauth",
        apiKeys: [],
      })}
      targetProviderId="system-chatgpt-subscription"
    />,
  );

  expect(screen.queryByText("Daily budget")).toBeNull();
});

it("places a targeted budget editor after the keys for metered Providers", () => {
  render(
    <SystemProviderCard
      onRemove={vi.fn()}
      provider={provider({ dailyBudgetUsd: "10" })}
      targetProviderId="system-openrouter-api"
      onTargetHandled={vi.fn()}
    />,
  );

  const cardText = document.body.textContent ?? "";
  expect(screen.getByTestId("budget-system-openrouter-api")).toHaveAttribute(
    "data-targeted",
    "true",
  );
  expect(cardText.indexOf("OpenClaw")).toBeLessThan(
    cardText.indexOf("Daily budget"),
  );
});
