import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { RouterSettings } from "./RouterSettings";
import { routerApi } from "@/lib/api/router";

vi.mock("@/lib/api/router", () => ({
  routerApi: {
    list: vi.fn(),
    save: vi.fn(),
    remove: vi.fn(),
    routes: vi.fn(),
    saveRoutes: vi.fn(),
    mode: vi.fn(),
    setMode: vi.fn(),
    pointer: vi.fn(),
    enable: vi.fn(),
    disconnect: vi.fn(),
    usage: vi.fn(),
  },
}));
vi.mock("@/lib/api/usageDashboard", () => ({
  usageDashboardApi: { listProviders: async () => [] },
}));
const provider = {
  id: "primary",
  displayName: "Primary",
  baseUrl: "http://localhost:9090/v1",
  wireApi: "responses" as const,
  priority: 1,
  enabled: true,
  authKind: "none" as const,
  credentialKeyId: null,
};
function mount() {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <RouterSettings onManageKeys={vi.fn()} />
    </QueryClientProvider>,
  );
}
beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { router: en.router });
  await i18n.changeLanguage("en");
});
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(routerApi.list).mockResolvedValue([provider]);
  vi.mocked(routerApi.mode).mockResolvedValue("auto");
  vi.mocked(routerApi.pointer).mockResolvedValue({
    state: "not_ours",
    current: "previous",
  });
  vi.mocked(routerApi.routes).mockResolvedValue([
    { logicalModel: "requested", upstreamModel: "upstream" },
  ]);
  vi.mocked(routerApi.usage).mockResolvedValue([]);
});
describe("Local routing settings", () => {
  it("loads existing mappings, edits them and persists the provider reference", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    const input = await screen.findByRole("textbox", {
      name: "Provider model 1",
    });
    expect(input).toHaveValue("upstream");
    await user.clear(input);
    await user.type(input, "new-model");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(routerApi.saveRoutes).toHaveBeenCalledWith("primary", [
        { logicalModel: "requested", upstreamModel: "new-model" },
      ]),
    );
    expect(routerApi.save).toHaveBeenCalledWith(provider);
    expect(routerApi.enable).not.toHaveBeenCalled();
  });
  it("does not change Codex until the explicit confirmation is accepted", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(
      await screen.findByRole("button", { name: "Connect Codex" }),
    );
    expect(routerApi.enable).not.toHaveBeenCalled();
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("config.toml");
    await user.click(within(dialog).getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(routerApi.enable).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Restart Codex",
    );
  });
  it("retains edits and reports errors if mapping persistence fails", async () => {
    vi.mocked(routerApi.saveRoutes).mockRejectedValue(new Error("Disk full"));
    const user = userEvent.setup();
    mount();
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    await screen.findByRole("textbox", { name: "Provider model 1" });
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
    expect(
      screen.getByRole("textbox", { name: "Provider model 1" }),
    ).toHaveValue("upstream");
  });
  it("rejects duplicate requested models before changing a provider", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    await screen.findByRole("textbox", { name: "Provider model 1" });
    await user.click(screen.getByRole("button", { name: "Add mapping" }));
    await user.type(
      screen.getByRole("textbox", { name: "Requested model 2" }),
      "requested",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Provider model 2" }),
      "another",
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "must be unique",
    );
    expect(routerApi.save).not.toHaveBeenCalled();
  });
  it("offers explicit disconnect and labels the usage lower bound", async () => {
    vi.mocked(routerApi.pointer).mockResolvedValue({
      state: "ours",
      current: null,
    });
    const user = userEvent.setup();
    mount();
    await user.click(
      await screen.findByRole("button", { name: "Restore direct connection" }),
    );
    expect(routerApi.disconnect).not.toHaveBeenCalled();
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Confirm",
      }),
    );
    await waitFor(() => expect(routerApi.disconnect).toHaveBeenCalledTimes(1));
    expect(
      screen.getByText(/lower bound, not your account balance/),
    ).toBeInTheDocument();
  });
});
