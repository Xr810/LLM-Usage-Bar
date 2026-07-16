import App from "./App";
import { TrayUsagePopover } from "@/components/tray-usage/TrayUsagePopover";

export function WindowSurface({ windowLabel }: { windowLabel: string }) {
  return windowLabel === "tray-popover" ? <TrayUsagePopover /> : <App />;
}
