import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

const fixedProviders = [
  ["system-chatgpt-subscription", "ChatGPT Plus/Pro", "chatgpt-subscription"],
  ["system-claude-subscription", "Claude Pro/Max", "claude-subscription"],
  ["system-openai-api", "OpenAI API", "openai-api"],
  ["system-anthropic-api", "Anthropic API", "anthropic-api"],
  ["system-openrouter-api", "OpenRouter", "openrouter-api"],
  ["system-gemini-api", "Google Gemini API", "gemini-api"],
  ["system-xai-api", "xAI (Grok) API", "xai-api"],
  ["system-deepseek-api", "DeepSeek API", "deepseek-api"],
  ["system-kimi-api", "Kimi / Moonshot API", "kimi-api"],
  ["system-glm-api", "GLM / Z.AI API", "glm-api"],
  ["system-qwen-api", "Qwen / DashScope API", "qwen-api"],
  ["system-minimax-api", "MiniMax API", "minimax-api"],
  ["system-mistral-api", "Mistral AI API", "mistral-api"],
  ["system-groq-api", "Groq API", "groq-api"],
  ["system-together-api", "Together AI", "together-api"],
  ["system-fireworks-api", "Fireworks AI", "fireworks-api"],
  ["system-perplexity-api", "Perplexity API", "perplexity-api"],
  ["system-siliconflow-api", "SiliconFlow API", "siliconflow-api"],
  ["system-nvidia-nim-api", "NVIDIA NIM API", "nvidia-nim-api"],
  ["system-cerebras-api", "Cerebras API", "cerebras-api"],
].map(([id, name, systemPresetKey]) => ({
  id,
  name,
  systemPresetKey,
  billingKind: systemPresetKey.endsWith("subscription")
    ? "subscription"
    : "metered",
  dailyBudgetUsd: null,
  enabled: true,
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageProviders: () => ({
    data: [
      {
        id: "custom",
        name: "Custom Example",
        systemPresetKey: null,
        billingKind: "metered",
        dailyBudgetUsd: "5",
        enabled: true,
      },
      {
        id: "custom-subscription",
        name: "Custom Subscription",
        systemPresetKey: null,
        billingKind: "subscription",
        dailyBudgetUsd: null,
        enabled: true,
      },
      ...fixedProviders.slice().reverse(),
    ],
    isLoading: false,
    error: null,
  }),
  useSaveUsageProvider: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteUsageProvider: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/lib/query/trayUsage", () => ({
  useApiBudgetConfig: () => ({
    data: { mode: "per_provider", sharedDailyBudgetUsd: null },
    isLoading: false,
    error: null,
  }),
  useSetApiBudgetConfig: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("./SystemProviderCard", () => ({
  SystemProviderCard: ({
    provider,
    targetProviderId,
    showBudget,
  }: {
    provider: { id: string; name: string };
    targetProviderId?: string;
    showBudget?: boolean;
  }) => (
    <div
      data-testid="fixed-provider-card"
      data-provider-id={provider.id}
      data-targeted={targetProviderId === provider.id}
      data-show-budget={showBudget}
    >
      {provider.name}
    </div>
  ),
}));

vi.mock("./ProviderDailyBudgetField", () => ({
  ProviderDailyBudgetField: ({
    providerId,
    targeted,
  }: {
    providerId: string;
    targeted?: boolean;
  }) => (
    <div
      data-testid={`custom-budget-${providerId}`}
      data-targeted={targeted}
      className="budget-field"
    />
  ),
}));

vi.mock("./ProviderModelPricingSection", () => ({
  ProviderModelPricingSection: ({ providerId }: { providerId: string }) => (
    <div
      data-testid={`custom-pricing-${providerId}`}
      className="pricing-section"
    />
  ),
}));

vi.mock("./OfficialPricingRefreshSection", () => ({
  OfficialPricingRefreshSection: () => <div>Official prices section</div>,
}));

vi.mock("@/components/usage-dashboard/UsageProviderDialog", () => ({
  UsageProviderDialog: () => null,
}));

it("renders the built-in Provider catalog in canonical order before custom Providers", () => {
  render(<UsageProvidersSettings />);

  expect(
    screen
      .getAllByTestId("fixed-provider-card")
      .map((card) => card.textContent),
  ).toEqual([
    "ChatGPT Plus/Pro",
    "Claude Pro/Max",
    "OpenAI API",
    "Anthropic API",
    "OpenRouter",
    "Google Gemini API",
    "xAI (Grok) API",
    "DeepSeek API",
    "Kimi / Moonshot API",
    "GLM / Z.AI API",
    "Qwen / DashScope API",
    "MiniMax API",
    "Mistral AI API",
    "Groq API",
    "Together AI",
    "Fireworks AI",
    "Perplexity API",
    "SiliconFlow API",
    "NVIDIA NIM API",
    "Cerebras API",
  ]);

  const pageText = document.body.textContent ?? "";
  expect(pageText.indexOf("Cerebras API")).toBeLessThan(
    pageText.indexOf("Custom Example"),
  );
  expect(
    screen.queryByRole("button", { name: "Edit ChatGPT Plus/Pro" }),
  ).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Delete ChatGPT Plus/Pro" }),
  ).toBeNull();
  expect(
    screen.getByRole("button", { name: "Edit Custom Example" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Delete Custom Example" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Add Provider" }),
  ).toBeInTheDocument();
});

it("searches the Provider catalog by name and preset identity", () => {
  render(<UsageProvidersSettings />);

  const search = screen.getByRole("searchbox", { name: "Search Providers" });
  fireEvent.change(search, { target: { value: "moonshot" } });

  expect(screen.getByText("Kimi / Moonshot API")).toBeInTheDocument();
  expect(screen.queryByText("OpenAI API")).toBeNull();
  expect(screen.queryByText("Custom Example")).toBeNull();

  fireEvent.change(search, { target: { value: "not-a-provider" } });
  expect(
    screen.getByText("No Providers match this search."),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Add Provider" }),
  ).toBeInTheDocument();
});

it("forwards fixed and custom targets while keeping custom budget editors full-width and metered-only", () => {
  const onTargetHandled = vi.fn();
  const { rerender } = render(
    <UsageProvidersSettings
      targetProviderId="system-openrouter-api"
      onTargetHandled={onTargetHandled}
    />,
  );

  const fixedOpenRouter = screen
    .getAllByTestId("fixed-provider-card")
    .find(
      (card) =>
        card.getAttribute("data-provider-id") === "system-openrouter-api",
    );
  expect(fixedOpenRouter).toHaveAttribute("data-targeted", "true");

  rerender(
    <UsageProvidersSettings
      targetProviderId="custom"
      onTargetHandled={onTargetHandled}
    />,
  );
  const customBudget = screen.getByTestId("custom-budget-custom");
  expect(customBudget).toHaveAttribute("data-targeted", "true");
  expect(customBudget.parentElement).toHaveClass("w-full");
  expect(screen.queryByTestId("custom-budget-custom-subscription")).toBeNull();
});
