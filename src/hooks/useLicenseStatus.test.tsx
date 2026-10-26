import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../tests/msw/server";
import {
  COMMUNITY_FOLLOWS_FREE_LIMITS,
  fetchLicenseStatus,
  isDashboardLimited,
  useLicenseStatus,
} from "./useLicenseStatus";
import { licenseStatus } from "../../tests/licenseStatus";

function Probe() {
  const license = useLicenseStatus();
  return (
    <div>
      <span data-testid="pending">{String(license.isPending)}</span>
      <span data-testid="error">{String(license.isError)}</span>
      <span data-testid="build">{license.data?.build ?? ""}</span>
      <span data-testid="limited">
        {String(
          isDashboardLimited(
            license.isPending || license.isError ? undefined : license.data,
          ),
        )}
      </span>
    </div>
  );
}

describe("isDashboardLimited", () => {
  it("restricts official free and community when the flag is on", () => {
    expect(COMMUNITY_FOLLOWS_FREE_LIMITS).toBe(true);
    expect(isDashboardLimited(licenseStatus("free"))).toBe(true);
    expect(isDashboardLimited(licenseStatus("free", "community"))).toBe(true);
    expect(isDashboardLimited(licenseStatus("pro", "community"), true)).toBe(
      true,
    );
  });

  it("leaves official pro and pro_p3 unrestricted, and community when the flag is off", () => {
    expect(isDashboardLimited(licenseStatus("pro"))).toBe(false);
    expect(isDashboardLimited(licenseStatus("pro_p3"))).toBe(false);
    expect(isDashboardLimited(licenseStatus("free", "community"), false)).toBe(
      false,
    );
    expect(isDashboardLimited(licenseStatus("pro", "community"), false)).toBe(
      false,
    );
  });

  it("treats loading and errors as restricted even when community would not be", () => {
    expect(isDashboardLimited(undefined, false)).toBe(true);
    expect(isDashboardLimited(null, false)).toBe(true);
  });
});

describe("fetchLicenseStatus", () => {
  it("returns the community mock when get_license_status is not wired", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.text("Command get_license_status not found", {
          status: 404,
        }),
      ),
    );

    await expect(fetchLicenseStatus()).resolves.toMatchObject({
      build: "community",
      tier: "free",
      features: [],
    });
  });

  it("rejects a real license failure instead of substituting the mock", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.text("license backend down", { status: 500 }),
      ),
    );

    await expect(fetchLicenseStatus()).rejects.toThrow(/license backend down/);
  });
});

describe("useLicenseStatus", () => {
  it("reports an error as restricted", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.text("license backend down", { status: 500 }),
      ),
    );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={client}>
        <Probe />
      </QueryClientProvider>,
    );

    await waitFor(() =>
      expect(screen.getByTestId("error")).toHaveTextContent("true"),
    );
    expect(screen.getByTestId("limited")).toHaveTextContent("true");
    expect(screen.getByTestId("build")).toHaveTextContent("");
  });
});
