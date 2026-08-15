import App from "./App";
import { TrayUsagePopover } from "@/components/tray-usage/TrayUsagePopover";

export type WindowSurfaceName = "main-window" | "tray-popover";

export function resolveWindowSurface(windowLabel: string): WindowSurfaceName {
  return windowLabel === "tray-popover" ? "tray-popover" : "main-window";
}

export function WindowSurface({ windowLabel }: { windowLabel: string }) {
  return resolveWindowSurface(windowLabel) === "tray-popover" ? (
    <TrayUsagePopover />
  ) : (
    <App />
  );
}
