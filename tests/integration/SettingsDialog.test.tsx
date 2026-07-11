import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

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
  it("loads the real usage-only surface through Tauri MSW", async () => {
    renderSettings();
    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getAllByText("Metered API")).not.toHaveLength(0);
    expect(screen.getByText("Static routes")).toBeInTheDocument();
  });

  it.each(["general", "proxy", "advanced", "mcp", "about"])(
    "maps historical tab %s to usage",
    async (defaultTab) => {
      renderSettings(true, defaultTab);
      expect(
        await screen.findAllByText("Official Subscription"),
      ).not.toHaveLength(0);
    },
  );

  it("does not mount dashboard commands while closed", () => {
    renderSettings(false);
    expect(screen.queryByText("Usage dashboard")).not.toBeInTheDocument();
  });
});
