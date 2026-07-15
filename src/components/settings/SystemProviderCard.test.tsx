import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { SystemProviderCard } from "./SystemProviderCard";
import type { UsageProviderView } from "@/types/usageDashboard";

vi.mock("./SystemProviderAgentBindings", () => ({
  SystemProviderAgentBindings: () => <div>Agent bindings</div>,
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
