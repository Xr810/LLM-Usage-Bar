import { invoke } from "@tauri-apps/api/core";
import type {
  MainWindowDestination,
  TrayUsageSnapshot,
} from "@/types/trayUsage";

export const getTrayUsageSnapshot = () =>
  invoke<TrayUsageSnapshot>("get_tray_usage_snapshot");

let refreshInFlight: Promise<TrayUsageSnapshot> | null = null;

export const refreshTrayUsage = () => {
  if (!refreshInFlight) {
    refreshInFlight = invoke<TrayUsageSnapshot>("refresh_tray_usage").finally(
      () => {
        refreshInFlight = null;
      },
    );
  }
  return refreshInFlight;
};

export const hideTrayPopover = () => invoke<void>("hide_tray_popover");

export const openMainFromTray = (destination: MainWindowDestination) =>
  invoke<void>("open_main_from_tray", { destination });

export const takePendingMainWindowDestination = () =>
  invoke<MainWindowDestination | null>("take_pending_main_window_destination");

export const quitFromTray = () => invoke<void>("quit_from_tray");
