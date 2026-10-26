import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type {
  AgentUsageBreakdownView,
  ModelProductGroupView,
  ModelUsageDashboardView,
} from "@/types/usageDashboard";
import { AgentBreakdownPage } from "./AgentBreakdownPage";
import { ModelUsagePage } from "./ModelUsagePage";
import {
  hasProRequiredWarning,
  visibleUsageWarnings,
} from "./proRequiredBreakdown";

const counts = { upstream: 2, estimated: 0, unavailable: 0 };

function modelGroup(
  models: ModelProductGroupView["models"] = [],
): ModelProductGroupView {
  return {
    productGroupId: "team-plan",
    billingKind: "metered",
    providerIds: ["team"],
    providerNames: ["Team"],
    eventCount: 2,
    inputTokens: 10,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens: 10,
    totalCostUsd: "0.50",
    costSourceCounts: counts,
    models,
  };
}

function modelDashboard(
  overrides: Partial<ModelUsageDashboardView> = {},
): ModelUsageDashboardView {
  return {
    startAt: 1,
    endAt: 2,
    totalTokens: 10,
    totalEventCount: 2,
    totalCostUsd: "0.50",
    productGroups: [modelGroup()],
    models: [],
    warnings: ["PRO_REQUIRED", "Kept a separate notice."],
    clampedFrom: "2026-09-27",
    ...overrides,
  };
}

function agentBreakdown(
  overrides: Partial<AgentUsageBreakdownView> = {},
): AgentUsageBreakdownView {
  return {
    startAt: 1,
    endAt: 2,
    totalTokens: 10,
    totalEventCount: 2,
    totalCostUsd: "0.50",
    agents: [],
    warnings: ["PRO_REQUIRED"],
    clampedFrom: "2026-09-27",
    ...overrides,
  };
}

function expectNoPurchase() {
  expect(screen.queryByText("PRO_REQUIRED")).toBeNull();
  expect(screen.queryByRole("link")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
  expect(
    screen.queryByText(/upgrade|purchase|subscribe|start trial/i),
  ).toBeNull();
}

describe("PRO_REQUIRED warning copy", () => {
  it("hides the authorization code and keeps other warnings", () => {
    expect(hasProRequiredWarning(["PRO_REQUIRED"])).toBe(true);
    expect(hasProRequiredWarning(["quota lagged"])).toBe(false);
    expect(visibleUsageWarnings(["PRO_REQUIRED", "quota lagged"])).toEqual([
      "quota lagged",
    ]);
  });
});

describe("model breakdown empty state", () => {
  it("keeps plan totals and explains the empty model list", async () => {
    const user = userEvent.setup();
    render(
      <ModelUsagePage dashboard={modelDashboard()} rangeLabel="30 days" />,
    );

    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
      "10",
    );
    expect(screen.getByTestId("breakdown-total-requests")).toHaveTextContent(
      "2",
    );
    expect(screen.getByTestId("breakdown-total-cost")).toHaveTextContent(
      "$0.50",
    );
    const group = screen.getByTestId("model-group-team-plan");
    expect(group).toHaveTextContent("10");
    expect(within(group).queryByRole("button")).toBeNull();
    expect(screen.getByTestId("model-list-empty")).toHaveTextContent(
      "Per-model breakdown isn't available for this range.",
    );
    expect(screen.getByText("Kept a separate notice.")).toBeInTheDocument();
    expectNoPurchase();

    await user.click(screen.getByRole("button", { name: "By model" }));
    expect(
      screen.getByText("Per-model breakdown isn't available for this range."),
    ).toBeInTheDocument();
    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
      "10",
    );
    expect(screen.queryByRole("img")).toBeNull();
    expectNoPurchase();
  });

  it("uses the no-usage empty state when the clamped range has no events", () => {
    render(
      <ModelUsagePage
        dashboard={modelDashboard({
          totalTokens: 0,
          totalEventCount: 0,
          totalCostUsd: "0",
          productGroups: [],
          warnings: ["PRO_REQUIRED"],
        })}
        rangeLabel="30 days"
      />,
    );

    expect(
      screen.getByText("No model usage was recorded in this range."),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Per-model breakdown isn't available for this range."),
    ).toBeNull();
    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent("0");
    expectNoPurchase();
  });
});

describe("agent breakdown empty state", () => {
  it("keeps range totals when the agent list is withheld", () => {
    render(
      <AgentBreakdownPage breakdown={agentBreakdown()} rangeLabel="30 days" />,
    );

    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
      "10",
    );
    expect(screen.getByTestId("breakdown-total-requests")).toHaveTextContent(
      "2",
    );
    expect(screen.getByTestId("breakdown-total-cost")).toHaveTextContent(
      "$0.50",
    );
    expect(
      screen.getByText("Per-agent breakdown isn't available for this range."),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("No Agent usage was recorded in this range."),
    ).toBeNull();
    expectNoPurchase();
  });

  it("uses the no-usage empty state when totals are zero", () => {
    render(
      <AgentBreakdownPage
        breakdown={agentBreakdown({
          totalTokens: 0,
          totalEventCount: 0,
          totalCostUsd: "0",
        })}
        rangeLabel="30 days"
      />,
    );

    expect(
      screen.getByText("No Agent usage was recorded in this range."),
    ).toBeInTheDocument();
    expect(screen.getByTestId("breakdown-total-requests")).toHaveTextContent(
      "0",
    );
    expectNoPurchase();
  });
});
