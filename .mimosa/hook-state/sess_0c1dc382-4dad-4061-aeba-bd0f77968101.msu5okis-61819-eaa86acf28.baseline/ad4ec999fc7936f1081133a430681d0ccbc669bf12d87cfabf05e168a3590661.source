import { invoke } from "@tauri-apps/api/core";

/** Rebuild the system tray menu after settings that affect it change. */
export async function updateTrayMenu(): Promise<void> {
  return invoke("update_tray_menu");
}
