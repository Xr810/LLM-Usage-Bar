import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  DashboardModuleView,
  ProviderUsageView,
  UsageProviderView,
} from "@/types/usageDashboard";
import type { ApiModuleProjection } from "./usageDashboardProjection";
import { ApiUsagePage } from "./ApiUsagePage";

vi.mock("@/lib/query/usageDashboard", () => ({
  useUsageEvents: () => ({
    data: { items: [], total: 0, page: 1, pageSize: 5 },
    error: null,
  }),
}));

const module: DashboardModuleView = {
  id: "api-stable-id",
  name: "Metered accounts",
  kind: "api",
  sortOrder: 9,
  visible: true,
  isSystem: true,
  providerCount: 2,
};

function provider(id: string): UsageProviderView {
  return {
    id,
    name: id === "azure" ? "Azure" : "OpenRouter",
    billingKind: "metered",
    productGroupId: "historical-product",
    tokenSources: ["proxy"],
    sessionSourceBindings: [],
    quotaSource: null,
    quotaIntervalSeconds: null,
    routeAppType: "codex",
    enabled: true,
    needsReview: false,
    createdAt: 1,
    updatedAt: 1,
    routeBaseUrl: "https://example.com",
    hasRouteCredentials: true,
    dashboardModuleId: null,
  };
}

function usage(id: string, cost: string | null): ProviderUsageView {
  return {
    provider: provider(id),
    eventCount: 3,
    inputTokens: 100,
    outputTokens: 20,
    cacheReadTokens: 10,
    cacheCreationTokens: 5,
    totalCostUsd: cost,
    costSourceCounts:
      cost == null
        ? { upstream: 0, estimated: 0, unavailable: 1 }
        : { upstream: 1, estimated: 0, unavailable: 0 },
    quota: null,
    quotaFetchState: null,
  };
}

function projection(
  overrides: Partial<ApiModuleProjection> = {},
): ApiModuleProjection {
  return {
    kind: "api",
    module,
    providers: [usage("azure", "1.25"), usage("openrouter", null)],
    totalTokens: 270,
    requestCount: 6,
    totalCostUsd: "1.25",
    costStatus: "partial",
    ...overrides,
  };
}

describe("ApiUsagePage", () => {
  it("renders all metered Providers with a metered-only summary", () => {
    render(<ApiUsagePage projection={projection()} startAt={10} endAt={20} />);

    expect(screen.getByText("Azure")).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();
    expect(screen.getByTestId("api-total-tokens")).toHaveTextContent("270");
    expect(screen.getByTestId("api-request-count")).toHaveTextContent("6");
    expect(screen.queryByText("Subscription")).toBeNull();
    expect(
      within(screen.getByTestId("metered-provider-azure")).getByText(
        "Upstream",
      ),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("metered-provider-openrouter")).getByText(
        "Cost unavailable",
      ),
    ).toBeInTheDocument();
  });

  it("labels partial known cost and never turns an unknown cost into zero", () => {
    render(<ApiUsagePage projection={projection()} startAt={10} endAt={20} />);

    expect(screen.getByTestId("api-total-cost")).toHaveTextContent("1.25");
    expect(screen.getByText("Partial cost data")).toBeInTheDocument();
    expect(screen.queryByText("$0")).toBeNull();
    expect(
      screen.getByText(
        "Cross-provider model distribution is unavailable until the desktop data service provides it.",
      ),
    ).toBeInTheDocument();
  });

  it("shows unavailable when no metered cost is known", () => {
    render(
      <ApiUsagePage
        projection={projection({
          providers: [usage("openrouter", null)],
          totalCostUsd: null,
          costStatus: "unavailable",
        })}
        startAt={10}
        endAt={20}
      />,
    );

    expect(screen.getByTestId("api-total-cost")).toHaveTextContent(
      "Cost unavailable",
    );
    expect(screen.queryByText("$0")).toBeNull();
  });

  it("labels an estimated Provider cost even when a numeric total exists", () => {
    const estimated = usage("azure", "0.42");
    estimated.costSourceCounts = {
      upstream: 0,
      estimated: 1,
      unavailable: 0,
    };
    render(
      <ApiUsagePage
        projection={projection({
          providers: [estimated],
          totalCostUsd: "0.42",
          costStatus: "estimated",
        })}
        startAt={10}
        endAt={20}
      />,
    );

    expect(
      within(screen.getByTestId("metered-provider-azure")).getByText(
        "Estimated",
      ),
    ).toBeInTheDocument();
  });
});
