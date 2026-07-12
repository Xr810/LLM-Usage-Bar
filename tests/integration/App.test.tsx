import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import App from "@/App";

const renderApp = () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
};

describe("App usage-dashboard acceptance with Tauri MSW", () => {
  it("loads a real provider-aware dashboard through the command boundary", async () => {
    const user = userEvent.setup();
    renderApp();

    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.queryByText("Metered API")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Metered usage" }));
    expect(await screen.findByText("Metered API")).toBeInTheDocument();
    expect(screen.getByText("Recent requests")).toBeInTheDocument();
    expect(await screen.findByText("claude-sonnet-4")).toBeInTheDocument();
  });
});
