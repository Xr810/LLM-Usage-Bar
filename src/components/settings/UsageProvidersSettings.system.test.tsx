import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

const fixedProviders = [
  ["system-chatgpt-subscription", "ChatGPT Plus/Pro", "chatgpt-subscription"],
  ["system-claude-subscription", "Claude Pro/Max", "claude-subscription"],
  ["system-openai-api", "OpenAI API", "openai-api"],
  ["system-anthropic-api", "Anthropic API", "anthropic-api"],
  ["system-openrouter-api", "OpenRouter", "openrouter-api"],
].map(([id, name, systemPresetKey]) => ({ id, name, systemPresetKey }));

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageProviders: () => ({
    data: [
      { id: "custom", name: "Custom Example", systemPresetKey: null },
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
  SystemProviderCard: ({ provider }: { provider: { name: string } }) => (
    <div data-testid="fixed-provider-card">{provider.name}</div>
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
