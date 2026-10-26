import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../../../tests/msw/server";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { primeOfficialProLicense } from "../../../tests/licenseStatus";
import { UsageDashboardPage } from "./UsageDashboardPage";

function renderPage() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  primeOfficialProLicense(client);
  return render(
    <QueryClientProvider client={client}>
      <UsageDashboardPage />
    </QueryClientProvider>,
  );
}

describe("UsageDashboardPage Provider-only contract", () => {
  it("preserves an open range draft and quota disclosure across a minute tick", async () => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(new Date("2026-10-15T12:00:00Z"));
    try {
      const fixture = await usageDashboardApi.getProviderDashboard(
        1,
        Date.now() / 1000,
      );
      fixture.providers[0].quota!.fiveHourPace = {
        status: "green",
        paceBasis: "measured",
        burnRatePercentPerHour: "12",
      };
      fixture.providers[0].quota!.fiveHourResetsAt = "2026-10-15T15:00:00Z";
      let finish: ((response: Response) => void) | undefined;
      server.use(
        http.post("http://tauri.local/get_provider_usage_dashboard", () =>
          HttpResponse.json(fixture),
        ),
      );
      const user = userEvent.setup();
      renderPage();
      await user.click(
        await screen.findByRole("button", { name: "Why this colour?" }),
      );
      await user.click(screen.getByRole("button", { name: "Custom range" }));
      await screen.findByRole("dialog");
      const endTime = document.querySelectorAll(
        'input[type="time"]',
      )[1] as HTMLInputElement;
      expect(endTime).toBeInstanceOf(HTMLInputElement);
      fireEvent.change(endTime, { target: { value: "10:30" } });
      server.use(
        http.post(
          "http://tauri.local/get_provider_usage_dashboard",
          () =>
            new Promise<Response>((resolve) => {
              finish = resolve;
            }),
        ),
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await waitFor(() => expect(finish).toBeDefined());
      expect(
        screen.getByRole("button", { name: "common.confirm" }),
      ).toBeVisible();
      expect(document.querySelectorAll('input[type="time"]')[1]).toHaveValue(
        "10:30",
      );
      expect(
        screen.queryByRole("button", { name: "Why this colour?" }),
      ).toBeNull();
      finish!(HttpResponse.json(fixture));
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Why this colour?" }),
        ).toHaveAttribute("aria-expanded", "true"),
      );
      expect(
        screen.getByRole("button", { name: "common.confirm" }),
      ).toBeVisible();
      expect(document.querySelectorAll('input[type="time"]')[1]).toBe(endTime);
      expect(endTime).toHaveValue("10:30");
      finish = undefined;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await waitFor(() => expect(finish).toBeDefined());
      finish!(HttpResponse.text("Live refresh unavailable", { status: 500 }));
      await waitFor(() =>
        expect(screen.getByText("Live refresh unavailable")).toBeVisible(),
      );
      expect(document.querySelectorAll('input[type="time"]')[1]).toHaveValue(
        "10:30",
      );
      server.use(
        http.post("http://tauri.local/get_provider_usage_dashboard", () =>
          HttpResponse.json(fixture),
        ),
      );
      await user.click(screen.getByRole("button", { name: "common.confirm" }));
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Why this colour?" }),
        ).toHaveAttribute("aria-expanded", "true"),
      );
      expect(screen.getByText("12%/h")).toBeVisible();
      const args = commandCalls("get_provider_usage_dashboard").at(-1)![1]!;
      expect(new Date(Number(args.endAt) * 1000).getHours()).toBe(10);
      expect(new Date(Number(args.endAt) * 1000).getMinutes()).toBe(30);
    } finally {
      vi.useRealTimers();
    }
  });

  it("renders subscription and metered Provider accounts without Agent navigation", async () => {
    renderPage();

    expect(
      await screen.findByRole("heading", { name: "Provider monitoring" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("ChatGPT")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Remaining quota" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Metered accounts" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("OpenRouter")).toHaveLength(2);
    expect(
      screen.getByRole("heading", { name: "Daily activity" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Usage trend" }),
    ).toBeInTheDocument();
    const rangeControls = screen.getByRole("group", {
      name: /time range/i,
    });
    expect(
      within(rangeControls).getByRole("button", { name: "Today" }),
    ).toBeInTheDocument();
    expect(
      within(rangeControls).getByRole("button", { name: "1 year" }),
    ).toBeInTheDocument();
    expect(
      within(rangeControls).getByRole("button", { name: "Custom range" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Daily")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Token usage by day" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "Agents" })).toBeNull();
  });

  it("queries exact Provider-wide ranges for today, 7 days, 30 days, and 1 year", async () => {
    const user = userEvent.setup();
    renderPage();
    await screen.findByText("ChatGPT");

    await user.click(screen.getByRole("button", { name: "Today" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(1),
    );
    expect(await screen.findByText("Hourly")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(2),
    );
    expect(await screen.findByText("Daily")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "30 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(3),
    );
    const beforeYear = commandCalls("get_provider_usage_dashboard").length;
    await user.click(screen.getByRole("button", { name: "1 year" }));
    await waitFor(() =>
      expect(
        commandCalls("get_provider_usage_dashboard").length,
      ).toBeGreaterThan(beforeYear),
    );
    expect(screen.getByTestId("usage-trend-range")).toHaveTextContent("1 year");

    for (const call of commandCalls("get_provider_usage_dashboard")) {
      const args = call[1] ?? {};
      expect(args).not.toHaveProperty("agentModuleId");
      expect(Number(args.endAt)).toBeGreaterThan(Number(args.startAt));
    }

    const oneYearCall = commandCalls("get_provider_usage_dashboard")
      .slice(beforeYear)
      .find((call) => {
        const args = call[1] ?? {};
        const duration = Number(args.endAt) - Number(args.startAt);
        return duration > 364 * 24 * 60 * 60 && duration < 366 * 24 * 60 * 60;
      });
    const oneYearArgs = oneYearCall?.[1] ?? {};
    const oneYearDuration =
      Number(oneYearArgs.endAt) - Number(oneYearArgs.startAt);
    expect(oneYearDuration).toBeGreaterThan(364 * 24 * 60 * 60);
    expect(oneYearDuration).toBeLessThan(366 * 24 * 60 * 60);

    const activityCalls = commandCalls("get_provider_usage_activity");
    expect(activityCalls).toHaveLength(1);
    const activityArgs = activityCalls[0]?.[1] ?? {};
    expect(activityArgs).not.toHaveProperty("agentModuleId");
    expect(
      Number(activityArgs.endAt) - Number(activityArgs.startAt),
    ).toBeGreaterThan(300 * 24 * 60 * 60);
  });

  it("does not expose manual Provider actions in the monitoring dashboard", async () => {
    renderPage();
    await screen.findByText("ChatGPT");

    expect(screen.queryByRole("button", { name: "Refresh quota" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Sync sessions" })).toBeNull();
    expect(commandCalls("refresh_provider_quota")).toHaveLength(0);
    expect(commandCalls("sync_provider_session_usage")).toHaveLength(0);
  });
});
