import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
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
    renderApp();

    expect(
      await screen.findAllByText("Official Subscription"),
    ).not.toHaveLength(0);
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.getAllByText("Metered API")).not.toHaveLength(0);
    expect(screen.getByText("Recent requests")).toBeInTheDocument();
    expect(screen.getByText("claude-sonnet-4")).toBeInTheDocument();
  });
});
