import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LicenseStatus } from "@/components/lub/licenseMock";
import { licenseStatusQueryKey } from "@/hooks/useLicenseStatus";
import * as licenseHook from "@/hooks/useLicenseStatus";
import { freeWindowStart } from "@/lib/usageRange";
import { resolveProviderActivityRange } from "@/lib/usageRange";
import { server } from "../../../tests/msw/server";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { licenseStatus } from "../../../tests/licenseStatus";
import { UsageDashboardPage } from "./UsageDashboardPage";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function renderDashboard(status?: LicenseStatus) {
  if (status) {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json(status),
      ),
    );
  }
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  if (status) client.setQueryData(licenseStatusQueryKey, status);
  return render(
    <QueryClientProvider client={client}>
      <UsageDashboardPage />
    </QueryClientProvider>,
  );
}

async function expectRestrictedDashboard() {
  expect(
    await screen.findByRole("heading", { name: "Provider monitoring" }),
  ).toBeInTheDocument();
  await waitFor(() =>
    expect(document.querySelector("[data-license-state]")).not.toHaveAttribute(
      "data-license-state",
      "pending",
    ),
  );
  const shell = document.querySelector(".usage-dashboard-shell");
  expect(shell).toHaveAttribute("data-dashboard-limited", "true");
  const rangeControls = screen.getByRole("group", { name: /time range/i });
  expect(
    within(rangeControls).getByRole("button", { name: "Today" }),
  ).toBeInTheDocument();
  expect(
    within(rangeControls).getByRole("button", { name: "7 days" }),
  ).toBeInTheDocument();
  expect(
    within(rangeControls).getByRole("button", { name: "30 days" }),
  ).toHaveAttribute("aria-pressed", "true");
  expect(
    within(rangeControls).queryByRole("button", { name: "1 year" }),
  ).toBeNull();
  expect(
    within(rangeControls).queryByRole("button", { name: "1d" }),
  ).toBeNull();
  expect(
    within(rangeControls).queryByRole("button", { name: "14d" }),
  ).toBeNull();
  expect(screen.queryByRole("tablist")).toBeNull();
  expect(screen.queryByText("PRO")).toBeNull();
  expect(commandCalls("get_model_usage_dashboard")).toHaveLength(0);
  expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);

  const boundary = Math.floor(freeWindowStart(Date.now()).getTime() / 1000);
  const dashboardCalls = commandCalls("get_provider_usage_dashboard");
  expect(dashboardCalls.length).toBeGreaterThan(0);
  for (const call of dashboardCalls) {
    expect(Number(call[1]?.startAt)).toBeGreaterThanOrEqual(boundary);
  }

  await waitFor(() =>
    expect(document.querySelectorAll("[data-activity-date]")).toHaveLength(365),
  );
  const weekLabels =
    document.querySelector("[data-activity-scroll] .mt-2")?.children.length ??
    0;
  expect(weekLabels).toBeGreaterThanOrEqual(52);
  expect(weekLabels).toBeLessThanOrEqual(54);
  const expectedActivity = resolveProviderActivityRange(Date.now());
  const activity = commandCalls("get_provider_usage_activity").at(-1)?.[1];
  expect(activity?.startAt).toBe(expectedActivity.startDate);
  expect(activity?.endAt).toBe(expectedActivity.endDate);
  expect(Number(activity?.endAt) - Number(activity?.startAt)).toBeGreaterThan(
    360 * 24 * 60 * 60,
  );
}

async function expectUnrestrictedDashboard() {
  expect(
    await screen.findByRole("heading", { name: "Provider monitoring" }),
  ).toBeInTheDocument();
  await waitFor(() =>
    expect(document.querySelector(".usage-dashboard-shell")).toHaveAttribute(
      "data-dashboard-limited",
      "false",
    ),
  );
  const rangeControls = screen.getByRole("group", { name: /time range/i });
  expect(
    within(rangeControls).getByRole("button", { name: "1 year" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("tab", { name: "Providers", selected: true }),
  ).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Models" })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Agents" })).toBeInTheDocument();
}

describe("UsageDashboardPage free and community limits", () => {
  it("restricts an official Free dashboard to 30 days without shrinking the heatmap", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 9, 26, 15, 0, 0));
    renderDashboard(licenseStatus("free"));
    await expectRestrictedDashboard();
    expect(
      (await screen.findAllByText("provider-model")).length,
    ).toBeGreaterThan(0);
  });

  it("follows Free limits for a community build while the flag is on", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 9, 26, 15, 0, 0));
    renderDashboard(licenseStatus("free", "community"));
    await expectRestrictedDashboard();
  });

  it("matches Pro for a community build when the flag is off", async () => {
    vi.spyOn(licenseHook, "communityFollowsFreeLimits").mockReturnValue(false);
    renderDashboard(licenseStatus("free", "community"));
    await expectUnrestrictedDashboard();
  });

  it.each(["pro", "pro_p3"] as const)(
    "keeps the %s dashboard on the existing presets and tabs",
    async (tier) => {
      renderDashboard(licenseStatus(tier));
      await expectUnrestrictedDashboard();
    },
  );

  it("renders the restricted dashboard while the license status is loading", async () => {
    let release: ((response: Response) => void) | undefined;
    server.use(
      http.post(
        "http://tauri.local/get_license_status",
        () =>
          new Promise<Response>((resolve) => {
            release = resolve;
          }),
      ),
    );
    renderDashboard();
    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(document.querySelector(".usage-dashboard-shell")).toHaveAttribute(
      "data-license-state",
      "pending",
    );
    expect(document.querySelector(".usage-dashboard-shell")).toHaveAttribute(
      "data-dashboard-limited",
      "true",
    );
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("button", { name: "1 year" })).toBeNull();
    expect(commandCalls("get_model_usage_dashboard")).toHaveLength(0);
    expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);
    release?.(HttpResponse.json(licenseStatus("pro")) as unknown as Response);
  });

  it("renders the restricted dashboard when the license status fails", async () => {
    vi.spyOn(licenseHook, "communityFollowsFreeLimits").mockReturnValue(false);
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.text("license backend down", { status: 500 }),
      ),
    );
    renderDashboard();
    await waitFor(() =>
      expect(document.querySelector(".usage-dashboard-shell")).toHaveAttribute(
        "data-license-state",
        "error",
      ),
    );
    expect(document.querySelector(".usage-dashboard-shell")).toHaveAttribute(
      "data-dashboard-limited",
      "true",
    );
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("button", { name: "1 year" })).toBeNull();
    expect(commandCalls("get_model_usage_dashboard")).toHaveLength(0);
  });
});
