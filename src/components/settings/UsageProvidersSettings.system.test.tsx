import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

const fixedProviders = [
  ["system-chatgpt-subscription", "ChatGPT Plus/Pro", "chatgpt-subscription"],
  ["system-claude-subscription", "Claude Pro/Max", "claude-subscription"],
  ["system-openai-api", "OpenAI API", "openai-api"],
  ["system-anthropic-api", "Anthropic API", "anthropic-api"],
  ["system-openrouter-api", "OpenRouter", "openrouter-api"],
].map(([id, name, systemPresetKey]) => ({
  id,
  name,
  systemPresetKey,
  billingKind: systemPresetKey.endsWith("subscription")
    ? "subscription"
    : "metered",
  dailyBudgetUsd: null,
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
      },
      {
        id: "custom-subscription",
        name: "Custom Subscription",
        systemPresetKey: null,
        billingKind: "subscription",
        dailyBudgetUsd: null,
      },
      ...fixedProviders.slice().reverse(),
    ],
    isLoading: false,
    error: null,
  }),
  useSaveUsageProvider: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("./SystemProviderCard", () => ({
  SystemProviderCard: ({
    provider,
    targetProviderId,
  }: {
    provider: { id: string; name: string };
    targetProviderId?: string;
  }) => (
    <div
      data-testid="fixed-provider-card"
      data-provider-id={provider.id}
      data-targeted={targetProviderId === provider.id}
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

vi.mock("@/components/usage-dashboard/UsageProviderDialog", () => ({
  UsageProviderDialog: () => null,
}));

it("renders the five fixed Provider cards in canonical order before custom Providers", () => {
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
  ]);

  const pageText = document.body.textContent ?? "";
  expect(pageText.indexOf("OpenRouter")).toBeLessThan(
    pageText.indexOf("Custom Example"),
  );
  expect(
    screen.queryByRole("button", { name: "Edit ChatGPT Plus/Pro" }),
  ).toBeNull();
  expect(
    screen.getByRole("button", { name: "Edit Custom Example" }),
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
