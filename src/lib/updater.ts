import { getVersion } from "@tauri-apps/api/app";

export type UpdateChannel = "stable" | "beta";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

export const MANAGED_UPDATES_UNAVAILABLE_CODE =
  "managed_updates_unavailable" as const;

export type UpdateCheckResult = {
  status: "unavailable";
  code: typeof MANAGED_UPDATES_UNAVAILABLE_CODE;
};

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForUpdate(
  _opts: CheckOptions = {},
): Promise<UpdateCheckResult> {
  return {
    status: "unavailable",
    code: MANAGED_UPDATES_UNAVAILABLE_CODE,
  };
}
