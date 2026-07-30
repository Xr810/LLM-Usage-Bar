import { invoke } from "@tauri-apps/api/core";
import type {
  ApiBudgetConfig,
  ApiBudgetMode,
  MainWindowDestination,
  TrayUsageSnapshot,
} from "@/types/trayUsage";
import type { UsageProviderView } from "@/types/usageDashboard";

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

export const getApiBudgetConfig = () =>
  invoke<ApiBudgetConfig>("get_api_budget_config");

export const setApiBudgetConfig = (
  mode: ApiBudgetMode,
  sharedDailyBudgetUsd: string | null,
) =>
  invoke<ApiBudgetConfig>("set_api_budget_config", {
    mode,
    sharedDailyBudgetUsd,
  });

export const setProviderDailyBudget = (
  providerId: string,
  dailyBudgetUsd: string | null,
) =>
  invoke<UsageProviderView>("set_provider_daily_budget", {
    providerId,
    dailyBudgetUsd,
  });

export const quitFromTray = () => invoke<void>("quit_from_tray");
