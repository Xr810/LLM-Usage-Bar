import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  isInaccessible,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../../../tests/msw/server";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { primeOfficialProLicense } from "../../../tests/licenseStatus";
import { UsageDashboardPage } from "./UsageDashboardPage";
import { usageDashboardApi } from "@/lib/api/usageDashboard";

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

const openTab = async (name: string) => {
  const user = userEvent.setup();
  renderPage();
  await screen.findByText("ChatGPT");
  await user.click(screen.getByRole("tab", { name }));
  return user;
};

describe("usage breakdown dimensions", () => {
  it.each(["model grouping", "model expansion", "agent expansion"])(
    "preserves %s across minute ticks and manual custom-end changes without previous metrics",
    async (state) => {
      vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
      vi.setSystemTime(new Date("2026-10-15T12:00:00Z"));
      try {
        const agent = state === "agent expansion";
        const command = agent
          ? "get_agent_usage_breakdown"
          : "get_model_usage_dashboard";
        const fixture = await (agent
          ? usageDashboardApi.getAgentBreakdown(1, 2)
          : usageDashboardApi.getModelDashboard(1, 2));
        const user = await openTab(agent ? "Agents" : "Models");
        const rowId = agent
          ? "agent-row-claude-code"
          : "model-group-claude-subscription";
        await screen.findByTestId(rowId);
        if (state === "model grouping") {
          await user.click(screen.getByRole("button", { name: "By model" }));
        } else {
          await user.click(
            within(screen.getByTestId(rowId)).getByRole("button"),
          );
        }
        const before = commandCalls(command).at(-1)![1]!;
        let finish: (response: Response) => void = () => {};
        server.use(
          http.post(
            `http://tauri.local/${command}`,
            () =>
              new Promise<Response>((resolve) => {
                finish = resolve;
              }),
          ),
        );
        act(() => {
          vi.advanceTimersByTime(60_000);
        });
        await waitFor(() =>
          expect(commandCalls(command).at(-1)![1]!.endAt).toBe(
            Number(before.endAt) + 60,
          ),
        );
        expect(commandCalls(command).at(-1)![1]!.startAt).toBe(before.startAt);
        expect(screen.getByTestId("breakdown-total-tokens")).not.toBeVisible();
        expect(
          isInaccessible(screen.getByTestId("breakdown-total-tokens")),
        ).toBe(true);
        expect(screen.getByRole("button", { name: "7 days" })).toBeEnabled();
        finish(
          HttpResponse.json({
            ...fixture,
            startAt: before.startAt,
            endAt: Number(before.endAt) + 60,
            totalTokens: 34000,
          }),
        );
        await waitFor(() =>
          expect(
            screen.getByTestId("breakdown-total-tokens"),
          ).toHaveTextContent("34K"),
        );
        const expectReadingState = () => {
          if (state === "model grouping") {
            expect(
              screen.getByRole("button", { name: "By model" }),
            ).toHaveAttribute("aria-pressed", "true");
            expect(
              screen.getByTestId("model-total-claude-opus-5"),
            ).toBeInTheDocument();
          } else {
            expect(
              within(screen.getByTestId(rowId)).getByRole("button"),
            ).toHaveAttribute("aria-expanded", "true");
          }
        };
        expectReadingState();

        // First establish a fixed custom range, then edit only its end.
        let customEnd = 0;
        for (const [time, totalTokens] of [
          ["10:00", 51000],
          ["11:00", 68000],
        ] as const) {
          const callsBefore = commandCalls(command).length;
          await user.click(
            screen.getByRole("button", { name: "Custom range" }),
          );
          fireEvent.change(document.querySelectorAll('input[type="time"]')[1], {
            target: { value: time },
          });
          await user.click(
            screen.getByRole("button", { name: "common.confirm" }),
          );
          await waitFor(() =>
            expect(commandCalls(command)).toHaveLength(callsBefore + 1),
          );
          const args = commandCalls(command).at(-1)![1]!;
          expect(args.startAt).toBe(before.startAt);
          if (customEnd) expect(Number(args.endAt) - customEnd).toBe(3600);
          customEnd = Number(args.endAt);
          expect(
            screen.getByTestId("breakdown-total-tokens"),
          ).not.toBeVisible();
          if (time === "11:00") {
            finish(
              HttpResponse.text("Custom range unavailable", { status: 500 }),
            );
            await screen.findByRole("alert", {
              name: "Custom range unavailable",
            });
            expect(
              screen.getByTestId("breakdown-total-tokens"),
            ).not.toBeVisible();
            expect(document.querySelector(".animate-pulse")).toBeNull();
            expect(
              isInaccessible(screen.getByTestId("breakdown-total-tokens")),
            ).toBe(true);
            await user.click(screen.getByRole("button", { name: "Retry" }));
            await waitFor(() =>
              expect(commandCalls(command)).toHaveLength(callsBefore + 2),
            );
          }
          finish(HttpResponse.json({ ...fixture, ...args, totalTokens }));
          await waitFor(() =>
            expect(
              screen.getByTestId("breakdown-total-tokens"),
            ).toHaveTextContent(time === "10:00" ? "51K" : "68K"),
          );
          expectReadingState();
        }
        const callsAfterCustom = commandCalls(command).length;
        act(() => {
          vi.advanceTimersByTime(60_000);
        });
        expect(commandCalls(command)).toHaveLength(callsAfterCustom);
        expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
          "68K",
        );
      } finally {
        vi.useRealTimers();
      }
    },
  );

  it("keeps Provider range controls usable while a changed range is pending or failed", async () => {
    const user = userEvent.setup();
    renderPage();
    await screen.findByText("ChatGPT");
    let finish: (response: Response) => void = () => {};
    server.use(
      http.post(
        "http://tauri.local/get_provider_usage_dashboard",
        () =>
          new Promise<Response>((resolve) => {
            finish = resolve;
          }),
      ),
    );
    await user.click(screen.getByRole("button", { name: "7 days" }));
    expect(screen.getByText("ChatGPT")).not.toBeVisible();
    expect(screen.getByRole("button", { name: "Custom range" })).toBeEnabled();
    finish(HttpResponse.text("Range unavailable", { status: 500 }));
    await screen.findByRole("alert", { name: "Range unavailable" });
    expect(screen.getByRole("button", { name: "30 days" })).toBeEnabled();
    server.resetHandlers();
    await user.click(screen.getByRole("button", { name: "30 days" }));
    await screen.findByText("ChatGPT");
  });

  it.each([
    ["Models", "get_model_usage_dashboard", "model-group-claude-subscription"],
    ["Agents", "get_agent_usage_breakdown", "agent-row-claude-code"],
  ])(
    "recovers %s from a failed request without a permanent skeleton",
    async (tab, command, resultId) => {
      let finish: (response: Response) => void = () => {};
      server.use(
        http.post(
          `http://tauri.local/${command}`,
          () =>
            new Promise<Response>((resolve) => {
              finish = resolve;
            }),
        ),
      );
      const user = await openTab(tab);
      await waitFor(() => expect(commandCalls(command)).toHaveLength(1));
      expect(document.querySelector(".animate-pulse")).not.toBeNull();
      finish(
        HttpResponse.text("Simulated database unavailable", { status: 500 }),
      );
      await screen.findByRole("alert", {
        name: "Simulated database unavailable",
      });
      expect(document.querySelector(".animate-pulse")).toBeNull();
      expect(screen.getByRole("button", { name: "7 days" })).toBeEnabled();

      await user.click(screen.getByRole("button", { name: "Retry" }));
      await waitFor(() => expect(commandCalls(command)).toHaveLength(2));
      expect(document.querySelector(".animate-pulse")).not.toBeNull();
      expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
      finish(HttpResponse.text("Still unavailable", { status: 500 }));
      await screen.findByRole("alert", { name: "Still unavailable" });
      expect(document.querySelector(".animate-pulse")).toBeNull();

      server.resetHandlers();
      await user.click(screen.getByRole("button", { name: "Retry" }));
      expect(await screen.findByTestId(resultId)).toBeInTheDocument();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(document.querySelector(".animate-pulse")).toBeNull();
      expect(screen.getAllByRole("button", { name: "7 days" })).toHaveLength(1);
    },
  );

  it("keeps the model and agent dimensions idle until their tab is opened", async () => {
    renderPage();
    await screen.findByText("ChatGPT");

    expect(commandCalls("get_model_usage_dashboard")).toHaveLength(0);
    expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(0);
    expect(
      screen.getByRole("tab", { name: "Providers", selected: true }),
    ).toBeInTheDocument();
  });

  it("groups model usage by plan and drills into the models a plan served", async () => {
    const user = await openTab("Models");

    expect(
      await screen.findByRole("heading", { name: "By model" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("get_model_usage_dashboard")).toHaveLength(1),
    );

    const claude = await screen.findByTestId("model-group-claude-subscription");
    expect(within(claude).getByText("Claude Pro/Max")).toBeInTheDocument();
    expect(within(claude).getByText("$4.50")).toBeInTheDocument();
    // The plan's models stay collapsed until the row is opened.
    expect(within(claude).queryByText("claude-sonnet-5")).toBeNull();

    await user.click(within(claude).getByRole("button"));
    expect(within(claude).getByText("claude-sonnet-5")).toBeInTheDocument();
    expect(within(claude).getByText("claude-opus-5")).toBeInTheDocument();
  });

  it("merges one model across every account it ran on", async () => {
    const user = await openTab("Models");
    await screen.findByTestId("model-group-claude-subscription");

    await user.click(screen.getByRole("button", { name: "By model" }));

    const opus = await screen.findByTestId("model-total-claude-opus-5");
    expect(within(opus).getByText("claude-opus-5")).toBeInTheDocument();
    // Served by both the subscription and the metered account.
    expect(
      within(opus).getByText("Anthropic API · Claude Pro/Max"),
    ).toBeInTheDocument();
    expect(within(opus).getByText("$5.50")).toBeInTheDocument();
    expect(screen.getByTestId("breakdown-total-tokens")).toHaveTextContent(
      "17K",
    );
  });

  it("ranks agents and keeps unattributed usage in its own bucket", async () => {
    const user = await openTab("Agents");

    expect(
      await screen.findByRole("heading", { name: "By agent" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(1),
    );

    const rows = screen.getAllByTestId(/^agent-row-/);
    expect(rows.map((row) => row.dataset.testid)).toEqual([
      "agent-row-claude-code",
      "agent-row-codex",
      "agent-row-legacy-agent",
      "agent-row-unassigned",
    ]);
    expect(
      within(rows[2]).getByText("Archived", { selector: "*" }),
    ).toBeInTheDocument();
    expect(within(rows[3]).getAllByText("Unassigned").length).toBeGreaterThan(
      0,
    );

    await user.click(within(rows[1]).getByRole("button"));
    expect(within(rows[1]).getByText("Accounts")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Claude Pro/Max")).toBeInTheDocument();
  });

  it("shares one time range across every dimension", async () => {
    const user = await openTab("Models");
    await screen.findByTestId("model-group-claude-subscription");

    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() =>
      expect(
        commandCalls("get_model_usage_dashboard").length,
      ).toBeGreaterThanOrEqual(2),
    );
    expect(screen.getByTestId("breakdown-range-label")).toHaveTextContent("7d");

    await user.click(screen.getByRole("tab", { name: "Agents" }));
    await waitFor(() =>
      expect(commandCalls("get_agent_usage_breakdown")).toHaveLength(1),
    );

    const modelRange = commandCalls("get_model_usage_dashboard").at(-1)?.[1];
    const agentRange = commandCalls("get_agent_usage_breakdown").at(-1)?.[1];
    expect(agentRange?.startAt).toBe(modelRange?.startAt);
    expect(agentRange?.endAt).toBe(modelRange?.endAt);
  });
});
