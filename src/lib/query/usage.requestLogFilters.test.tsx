import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { isDashboardLimited, useLicenseStatus } from "@/hooks/useLicenseStatus";
import { licenseStatusQueryKey } from "@/hooks/useLicenseStatus";
import { useRequestLogs } from "@/lib/query/usage";
import { server } from "../../../tests/msw/server";
import { commandCalls } from "../../../tests/msw/tauriMocks";
import { licenseStatus } from "../../../tests/licenseStatus";

/**
 * The request-log table and its provider/model selects were removed from the
 * main window before this branch (RequestLogTable, d00e4cf6). What remains is
 * `useRequestLogs`. FE-15 must not license-gate that query: Free still sends
 * provider and model filters.
 */
function RequestLogFilters() {
  const license = useLicenseStatus();
  const limited = isDashboardLimited(
    license.isPending || license.isError ? undefined : license.data,
  );
  const logs = useRequestLogs({
    filters: { providerName: "OpenRouter", model: "gpt-4.1" },
    range: { preset: "30d" },
    options: { refetchInterval: false },
  });
  return (
    <form aria-label="Request log filters">
      <label>
        Provider
        <select aria-label="Request log provider" defaultValue="OpenRouter">
          <option>OpenRouter</option>
        </select>
      </label>
      <label>
        Model
        <select aria-label="Request log model" defaultValue="gpt-4.1">
          <option>gpt-4.1</option>
        </select>
      </label>
      <span data-testid="limited">{String(limited)}</span>
      <span data-testid="rows">{logs.data?.data.length ?? ""}</span>
    </form>
  );
}

describe("request log provider and model filters", () => {
  it("still renders and queries provider and model filters under Free", async () => {
    const free = licenseStatus("free");
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json(free),
      ),
      http.post("http://tauri.local/get_request_logs", () =>
        HttpResponse.json({ data: [], total: 0, page: 0, pageSize: 20 }),
      ),
    );
    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, refetchOnWindowFocus: false },
      },
    });
    client.setQueryData(licenseStatusQueryKey, free);
    render(
      <QueryClientProvider client={client}>
        <RequestLogFilters />
      </QueryClientProvider>,
    );

    expect(
      screen.getByRole("combobox", { name: "Request log provider" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("combobox", { name: "Request log model" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("limited")).toHaveTextContent("true");

    await waitFor(() =>
      expect(commandCalls("get_request_logs").length).toBeGreaterThan(0),
    );
    const filters = commandCalls("get_request_logs").at(-1)?.[1]?.filters as {
      providerName?: string;
      model?: string;
    };
    expect(filters.providerName).toBe("OpenRouter");
    expect(filters.model).toBe("gpt-4.1");
  });
});
