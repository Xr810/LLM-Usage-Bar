import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { usageDashboardApi } from "@/lib/api/usageDashboard";
import { proxyApi } from "@/lib/api/proxy";

const renderSettings = (open = true, defaultTab?: string) => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SettingsPage
        open={open}
        onOpenChange={() => {}}
        defaultTab={defaultTab}
      />
    </QueryClientProvider>,
  );
};

describe("SettingsPage integration", () => {
  it("loads three isolated configuration sections through Tauri MSW", async () => {
    const user = userEvent.setup();
    renderSettings();

    expect(
      await screen.findByDisplayValue("Personal usage"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(screen.queryByText("Proxy forwarding targets")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Provider" }));
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    expect(screen.getByText("Azure API")).toBeInTheDocument();
    expect(screen.getByText("OpenRouter")).toBeInTheDocument();
    expect(screen.queryByText("Proxy forwarding targets")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Proxy & routing" }));
    expect(
      await screen.findByText("Proxy forwarding targets"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "These targets decide where requests are forwarded. They are not a dashboard filter.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Official Subscription")).toBeNull();
    expect(
      screen.getByRole("button", { name: /^(Start|Stop) proxy$/ }),
    ).toBeInTheDocument();
  });

  it.each(["general", "advanced", "mcp", "about", "unknown"])(
    "maps historical or unknown tab %s to Usage modules",
    async (defaultTab) => {
      renderSettings(true, defaultTab);
      expect(
        await screen.findByDisplayValue("Personal usage"),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("tab", { name: "Usage modules" }),
      ).toHaveAttribute("aria-selected", "true");
    },
  );

  it("honors Provider and Proxy defaults", async () => {
    const providerView = renderSettings(true, "providers");
    expect(
      await screen.findByText("Official Subscription"),
    ).toBeInTheDocument();
    providerView.unmount();

    renderSettings(true, "proxy");
    expect(
      await screen.findByText("Proxy forwarding targets"),
    ).toBeInTheDocument();
  });

  it("does not start configuration queries while closed", () => {
    const modules = vi.spyOn(usageDashboardApi, "listDashboardModules");
    const providers = vi.spyOn(usageDashboardApi, "listProviders");
    const proxy = vi.spyOn(proxyApi, "isProxyRunning");

    renderSettings(false);

    expect(modules).not.toHaveBeenCalled();
    expect(providers).not.toHaveBeenCalled();
    expect(proxy).not.toHaveBeenCalled();
  });
});
