import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { SystemProviderCard } from "./SystemProviderCard";
import type { UsageProviderView } from "@/types/usageDashboard";

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
vi.mock("./ClaudeCliAuthSection", () => ({
  ClaudeCliAuthSection: () => <div>Claude auth</div>,
}));
vi.mock("@/components/providers/forms/CodexOAuthSection", () => ({
  CodexOAuthSection: () => <div>ChatGPT auth</div>,
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useSystemProviderCredentialActions: () => ({ isPending: false }),
  useSetUsageProviderEnabled: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

it("locks system identity and endpoint without Edit or Delete actions", () => {
  render(
    <SystemProviderCard
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
  expect(screen.getByText("https://api.openai.com/v1")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /edit/i })).toBeNull();
  expect(screen.queryByRole("button", { name: /delete/i })).toBeNull();
});

it("places a targeted budget editor after authentication for metered Providers", () => {
  const onTargetHandled = vi.fn();
  render(
    <SystemProviderCard
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
  expect(cardText.indexOf("Upstream API key required")).toBeLessThan(
    cardText.indexOf("Daily budget"),
  );
  expect(cardText).not.toContain("Agent bindings");
});

it("does not render a budget editor for subscription Providers", () => {
  render(
    <SystemProviderCard
      provider={
        {
          id: "system-chatgpt-subscription",
          name: "ChatGPT Plus/Pro",
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
