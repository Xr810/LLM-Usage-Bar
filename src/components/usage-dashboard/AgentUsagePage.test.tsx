import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentModuleView,
  ProviderUsageView,
  UsageProviderView,
} from "@/types/usageDashboard";
import type { AgentUsageProjection } from "./usageDashboardProjection";
import { AgentUsagePage } from "./AgentUsagePage";

const mocks = vi.hoisted(() => ({ events: vi.fn() }));

vi.mock("@/lib/query/usageDashboard", () => ({
  useProviderUsageEvents: (...args: unknown[]) => mocks.events(...args),
}));

const agent: AgentModuleView = {
  id: "codex",
  name: "Codex",
  sortOrder: 1,
  visible: true,
  isFixed: true,
  archivedAt: null,
  providerCount: 2,
};

function provider(
  id: string,
  billingKind: "subscription" | "metered",
): UsageProviderView {
  return {
    id,
    name: id === "sub" ? "Official Subscription" : "Azure API",
    billingKind,
    productGroupId: "codex",
    tokenSources: billingKind === "subscription" ? ["session_log"] : ["proxy"],
    sessionSourceBindings: billingKind === "subscription" ? ["codex"] : [],
    bindings: [],
    quotaSource: billingKind === "subscription" ? "codex" : null,
    quotaIntervalSeconds: billingKind === "subscription" ? 300 : null,
    dailyBudgetUsd: billingKind === "metered" ? "10" : null,
    routeAppType: billingKind === "metered" ? "codex" : null,
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: billingKind === "metered" ? "https://example.com" : null,
    hasRouteCredentials: billingKind === "metered",
    systemPresetKey: null,
    systemAuthKind: null,
    canonicalEndpoint: null,
    compatibleAgentModuleIds: [],
    upstreamCredentialStatus: "not_required",
    upstreamCredentialVersion: 0,
    canClearUpstreamCredential: false,
    lastConnectionTestAt: null,
    lastConnectionTestStatus: null,
  };
}

function usage(
  id: string,
  billingKind: "subscription" | "metered",
): ProviderUsageView {
  return {
    provider: provider(id, billingKind),
    sharedAccount: true,
    eventCount: billingKind === "subscription" ? 9 : 2,
    inputTokens: billingKind === "subscription" ? 999 : 100,
    outputTokens: 20,
    cacheReadTokens: 10,
    cacheCreationTokens: 5,
    totalCostUsd: billingKind === "metered" ? "1.25" : null,
    costSourceCounts:
      billingKind === "metered"
        ? { upstream: 1, estimated: 0, unavailable: 0 }
        : { upstream: 0, estimated: 0, unavailable: 1 },
    quota: null,
    quotaFetchState: null,
  };
}

function projection(
  overrides: Partial<AgentUsageProjection> = {},
): AgentUsageProjection {
  return {
    agent,
    subscriptionProviders: [usage("sub", "subscription")],
    meteredProviders: [usage("metered", "metered")],
    meteredTotalTokens: 135,
    meteredRequestCount: 2,
    meteredTotalCostUsd: "1.25",
    meteredCostStatus: "complete",
    ...overrides,
  };
}

describe("AgentUsagePage", () => {
  beforeEach(() => {
    mocks.events.mockReset().mockReturnValue({
      data: { items: [], total: 0, page: 1, pageSize: 5 },
      error: null,
    });
  });

  it("shows subscription and metered sections together with Agent-scoped events", () => {
    render(
      <AgentUsagePage
        projection={projection()}
        startAt={10}
        endAt={20}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
      />,
    );

    expect(screen.getByText("Subscription accounts")).toBeInTheDocument();
    expect(screen.getByText("Metered usage")).toBeInTheDocument();
    expect(screen.getByText("Official Subscription")).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.queryByText("Shared account quota")).toBeNull();
    expect(screen.queryByText("Shared account")).toBeNull();
    expect(screen.getByTestId("metered-total-tokens")).toHaveTextContent("135");
    expect(screen.getByTestId("metered-request-count")).toHaveTextContent("2");
    expect(mocks.events).toHaveBeenCalledWith("metered", 10, 20, 1, 5);

    for (const configuration of [
      "Add Provider",
      "Static routes",
      "Start proxy",
      "Stop proxy",
      "Enable",
      "Disable",
    ]) {
      expect(screen.queryByText(configuration)).toBeNull();
    }
  });

  it("opens Settings when an Agent has no Provider history", () => {
    const onOpenSettings = vi.fn();
    render(
      <AgentUsagePage
        projection={projection({
          subscriptionProviders: [],
          meteredProviders: [],
          meteredTotalTokens: 0,
          meteredRequestCount: 0,
          meteredTotalCostUsd: null,
          meteredCostStatus: "unavailable",
        })}
        startAt={10}
        endAt={20}
        onRefreshQuota={vi.fn()}
        onSyncSessions={vi.fn()}
        onOpenSettings={onOpenSettings}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Configure Providers in Settings" }),
    );
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });
});
