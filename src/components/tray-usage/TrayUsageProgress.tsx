import type { TrayUsageStatus } from "@/types/trayUsage";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { statusLabel, type TrayUsageTranslate } from "./trayUsagePresentation";

export function TrayUsageProgress({
  label,
  value,
  status,
}: {
  label: string;
  value: number;
  status: TrayUsageStatus;
}) {
  return (
    <progress
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value}
      value={value}
      max={100}
      className={cn("tray-usage-progress", `is-${status}`)}
    />
  );
}

export function TrayUsageStatusBadge({
  status,
  t,
  live = false,
}: {
  status: TrayUsageStatus;
  t: TrayUsageTranslate;
  live?: boolean;
}) {
  return (
    <Badge
      variant="outline"
      role={live ? "status" : undefined}
      className={cn("tray-usage-status", `is-${status}`)}
    >
      <span aria-hidden="true" className="tray-usage-status-dot" />
      {statusLabel(status, t)}
    </Badge>
  );
}
