import { invoke } from "@tauri-apps/api/core";
import type { ClaudeCliAuthStatus } from "@/types/usageDashboard";

export const claudeCliAuthApi = {
  getStatus: (): Promise<ClaudeCliAuthStatus> =>
    invoke("get_claude_cli_auth_status"),
  startLogin: (): Promise<void> => invoke("start_claude_cli_login"),
  logout: (): Promise<ClaudeCliAuthStatus> => invoke("logout_claude_cli"),
};
