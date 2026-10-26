import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import type { LicenseStatus } from "@/types/license";
import { licenseStatusQueryKey } from "@/hooks/useLicenseStatus";
import { server } from "../../../tests/msw/server";
import { licenseStatus } from "../../../tests/licenseStatus";
import { UsageDashboardPage } from "./UsageDashboardPage";

function bucket(totalTokens: number, eventCount: number, totalCostUsd: string) {
  return {
    startAt: 1_700_000_000,
    endAt: 1_700_086_400,
    eventCount,
    inputTokens: totalTokens,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens,
    totalCostUsd,
    costSourceCounts: { upstream: eventCount, estimated: 0, unavailable: 0 },
  };
}

function renderDashboard(status: LicenseStatus) {
  server.use(
    http.post("http://tauri.local/get_license_status", () =>
      HttpResponse.json(status),
    ),
  );
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  client.setQueryData(licenseStatusQueryKey, status);
  return render(
    <QueryClientProvider client={client}>
      <UsageDashboardPage onOpenSettings={vi.fn()} />
    </QueryClientProvider>,
  );
}

function redactProviderDashboard(options?: {
  tokens?: number;
  warnings?: string[];
}) {
  const tokens = options?.tokens ?? 10;
  server.use(
    http.post("http://tauri.local/get_provider_usage_dashboard", () =>
      HttpResponse.json({
        startAt: 1,
        endAt: 2,
        providers: [],
        trendGranularity: "day",
        trendBuckets: [
          bucket(tokens, tokens === 0 ? 0 : 2, tokens === 0 ? "0" : "0.50"),
        ],
        warnings: options?.warnings ?? ["PRO_REQUIRED"],
        clampedFrom: "2026-09-27",
      }),
    ),
  );
}

function expectNoPurchase() {
  expect(screen.queryByText("PRO_REQUIRED")).toBeNull();
  expect(screen.queryByRole("link")).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Configure Providers in Settings" }),
  ).toBeNull();
  expect(
    screen.queryByText(/upgrade|purchase|subscribe|start trial/i),
  ).toBeNull();
  expect(screen.queryByText("No Provider accounts are configured.")).toBeNull();
}

describe("Free provider dashboard with a withheld account split", () => {
  it.each([
    ["community", licenseStatus("free", "community")],
    [
      "community with upsellEntry on",
      { ...licenseStatus("free", "community"), uiFlags: { upsellEntry: true } },
    ],
    [
      "official Free with upsellEntry on",
      { ...licenseStatus("free"), uiFlags: { upsellEntry: true } },
    ],
  ])(
    "keeps trend totals and shows an empty account list for %s",
    async (_label, status) => {
      redactProviderDashboard();
      renderDashboard(status);

      expect(
        await screen.findByRole("img", { name: "Token usage by day" }),
      ).toBeInTheDocument();
      expect(screen.getByTestId("provider-total-tokens")).toHaveTextContent(
        "10",
      );
      expect(screen.getByTestId("provider-total-requests")).toHaveTextContent(
        "2",
      );
      expect(screen.getByTestId("provider-total-cost")).toHaveTextContent(
        "$0.50",
      );
      expect(
        screen.getByText(
          "Per-account breakdown isn't available for this range.",
        ),
      ).toBeInTheDocument();
      expect(screen.queryByRole("alert")).toBeNull();
      expectNoPurchase();
    },
  );

  it("still shows a non-authorization warning", async () => {
    redactProviderDashboard({
      warnings: ["PRO_REQUIRED", "Quota refresh lagged."],
    });
    renderDashboard(licenseStatus("free", "community"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Quota refresh lagged.",
    );
    expect(screen.queryByText("PRO_REQUIRED")).toBeNull();
    expectNoPurchase();
  });

  it("shows the chart empty state when the clamped window has no usage", async () => {
    redactProviderDashboard({ tokens: 0 });
    renderDashboard(licenseStatus("free", "community"));

    expect(
      await screen.findByText("No usage recorded in this range."),
    ).toBeInTheDocument();
    expect(screen.getByTestId("provider-total-tokens")).toHaveTextContent("0");
    expect(screen.getByTestId("provider-total-requests")).toHaveTextContent(
      "0",
    );
    expect(screen.getByTestId("provider-total-cost")).toHaveTextContent(
      "$0.00",
    );
    expect(
      screen.queryByText(
        "Per-account breakdown isn't available for this range.",
      ),
    ).toBeNull();
    expect(
      screen.queryByRole("img", { name: "Token usage by day" }),
    ).toBeNull();
    expectNoPurchase();
  });

  it("keeps the unconfigured empty state when the account list is genuinely empty", async () => {
    redactProviderDashboard({ warnings: [], tokens: 0 });
    renderDashboard(licenseStatus("pro"));

    expect(
      await screen.findByText("No Provider accounts are configured."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Configure Providers in Settings" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("provider-breakdown-empty")).toBeNull();
    await waitFor(() => expect(screen.queryByText("PRO_REQUIRED")).toBeNull());
  });
});
