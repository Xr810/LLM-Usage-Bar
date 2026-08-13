import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";

const mocks = vi.hoisted(() => ({
  login: vi.fn(),
  logout: vi.fn(),
  refetch: vi.fn(),
  statusData: {
    installed: true,
    authenticated: true,
    authMethod: "api_key" as "api_key" | "claude_account" | "other" | null,
    subscriptionType: null as "pro" | "max" | null,
    lastQuotaSampleAt: null as number | null,
    errorCode: null,
  },
  settingsData: { claudeOauthQuotaEnabled: false },
  saveSettings: vi.fn(),
}));
vi.mock("@/lib/query/usageDashboard", () => ({
  useClaudeCliAuthStatus: () => ({
    data: mocks.statusData,
    isLoading: false,
    refetch: mocks.refetch,
  }),
  useClaudeCliAuthActions: () => ({
    startLogin: mocks.login,
    logout: mocks.logout,
    isPending: false,
  }),
}));
vi.mock("@/lib/query", () => ({
  useSettingsQuery: () => ({
    data: mocks.settingsData,
    isLoading: false,
  }),
  useSaveSettingsMutation: () => ({
    mutate: mocks.saveSettings,
    isPending: false,
  }),
}));

describe("ClaudeCliAuthSection", () => {
  beforeEach(() => {
    mocks.statusData.authMethod = "api_key";
    mocks.statusData.subscriptionType = null;
    mocks.settingsData.claudeOauthQuotaEnabled = false;
    mocks.saveSettings.mockClear();
  });

  it("shows detected API key CLI state, reconnect, and logout", async () => {
    render(<ClaudeCliAuthSection />);
    expect(
      screen.getByText("Claude CLI detected · API Key"),
    ).toBeInTheDocument();
    expect(screen.getByText("No quota sample yet")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reconnect Claude" }));
    fireEvent.click(screen.getByRole("button", { name: "Disconnect Claude" }));
    await waitFor(() => expect(mocks.login).toHaveBeenCalledOnce());
    expect(mocks.logout).toHaveBeenCalledOnce();
  });

  it("reports when the local quota sample was taken", () => {
    // The line used to read "Quota unavailable" unconditionally, contradicting
    // the dashboard that was showing that very quota.
    mocks.statusData.lastQuotaSampleAt = Math.floor(Date.now() / 1000) - 180;
    render(<ClaudeCliAuthSection />);

    expect(screen.getByText(/Quota updated/)).toBeInTheDocument();
    expect(screen.queryByText("No quota sample yet")).toBeNull();
    mocks.statusData.lastQuotaSampleAt = null;
  });

  it("prefers the official subscription plan when one is available", () => {
    mocks.statusData.authMethod = "claude_account";
    mocks.statusData.subscriptionType = "max";
    render(<ClaudeCliAuthSection />);
    expect(screen.getByText("Claude CLI detected · Max")).toBeInTheDocument();
  });

  it("opens the consent dialog when enabling the keychain quota toggle", async () => {
    render(<ClaudeCliAuthSection />);
    const toggle = screen.getByRole("switch", {
      name: "Query official quota with Claude Code credentials",
    });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);

    expect(
      screen.getByText("Allow reading Claude Code's login?"),
    ).toBeInTheDocument();
    expect(mocks.saveSettings).not.toHaveBeenCalled();
  });

  it("requires the informed checkbox before enabling keychain access", async () => {
    render(<ClaudeCliAuthSection />);
    const toggle = screen.getByRole("switch", {
      name: "Query official quota with Claude Code credentials",
    });

    // 勾选前点 Allow:对话框关闭但不保存。
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    expect(mocks.saveSettings).not.toHaveBeenCalled();

    // 重新打开,勾选说明后点 Allow 才保存开启状态。
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledOnce());
    expect(mocks.saveSettings).toHaveBeenCalledWith({
      claudeOauthQuotaEnabled: true,
    });
  });

  it("disabling the toggle revokes consent without a dialog", async () => {
    mocks.settingsData.claudeOauthQuotaEnabled = true;
    render(<ClaudeCliAuthSection />);
    fireEvent.click(
      screen.getByRole("switch", {
        name: "Query official quota with Claude Code credentials",
      }),
    );
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledOnce());
    expect(mocks.saveSettings).toHaveBeenCalledWith({
      claudeOauthQuotaEnabled: false,
    });
    expect(
      screen.queryByText("Allow reading Claude Code's login?"),
    ).toBeNull();
  });
});
