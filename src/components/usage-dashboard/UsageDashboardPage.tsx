import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { UsageDateRangePicker } from "@/components/usage/UsageDateRangePicker";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import {
  useRefreshProviderQuota,
  useSyncProviderSessionUsage,
  useUsageDashboard,
} from "@/lib/query/usageDashboard";
import { resolveUsageRange } from "@/lib/usageRange";
import type { UsageRangeSelection } from "@/types/usage";
import type { DashboardModuleView } from "@/types/usageDashboard";
import { ApiUsagePage } from "./ApiUsagePage";
import { SubscriptionModulePage } from "./SubscriptionModulePage";
import { projectDashboardModule } from "./usageDashboardProjection";

interface UsageDashboardPageProps {
  selectedModule?: DashboardModuleView | null;
  onOpenSettings?: () => void;
}

export function UsageDashboardPage({
  selectedModule = null,
  onOpenSettings,
}: UsageDashboardPageProps) {
  const { t } = useTranslation();
  const [rangeClockMs, setRangeClockMs] = useState(() => Date.now());
  const advanceRangeClock = useCallback(() => setRangeClockMs(Date.now()), []);
  useUsageEventBridge(advanceRangeClock);
  useEffect(() => {
    const interval = globalThis.setInterval(advanceRangeClock, 30_000);
    return () => globalThis.clearInterval(interval);
  }, [advanceRangeClock]);
  const [selection, setSelection] = useState<UsageRangeSelection>({
    preset: "today",
  });
  const [warnings, setWarnings] = useState<string[]>([]);
  const [errors, setErrors] = useState<string[]>([]);
  const range = useMemo(
    () => resolveUsageRange(selection, rangeClockMs),
    [rangeClockMs, selection],
  );
  const dashboard = useUsageDashboard(range.startDate, range.endDate);
  const refreshQuota = useRefreshProviderQuota();
  const syncSession = useSyncProviderSessionUsage();
  const projection = useMemo(
    () =>
      selectedModule && dashboard.data
        ? projectDashboardModule(selectedModule, dashboard.data)
        : null,
    [dashboard.data, selectedModule],
  );

  const errorText = (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause);
  const run = async (operation: () => Promise<unknown>) => {
    try {
      await operation();
    } catch (cause) {
      setErrors((current) => [...current, errorText(cause)]);
    }
  };
  const sync = async (providerId: string) => {
    try {
      const result = await syncSession.mutateAsync(providerId);
      setWarnings(result.warnings ?? []);
      setErrors(result.errors ?? []);
    } catch (cause) {
      setErrors([errorText(cause)]);
    }
  };

  const queryErrors = dashboard.error ? [errorText(dashboard.error)] : [];

  return (
    <div className="space-y-4 pb-6">
      <div className="flex flex-wrap items-center justify-end gap-2">
        {(["today", "7d", "30d"] as const).map((preset) => (
          <Button
            key={preset}
            size="sm"
            variant={selection.preset === preset ? "default" : "outline"}
            aria-pressed={selection.preset === preset}
            onClick={() => setSelection({ preset })}
          >
            {preset === "today"
              ? t("usageDashboard.today", { defaultValue: "Today" })
              : preset === "7d"
                ? t("usageDashboard.sevenDays", { defaultValue: "7 days" })
                : t("usageDashboard.thirtyDays", {
                    defaultValue: "30 days",
                  })}
          </Button>
        ))}
        <UsageDateRangePicker
          selection={selection}
          onApply={setSelection}
          triggerLabel={t("usageDashboard.customRange", {
            defaultValue: "Custom range",
          })}
        />
      </div>

      {[...(dashboard.data?.warnings ?? []), ...warnings].map((warning) => (
        <Alert key={warning}>
          <AlertDescription>{warning}</AlertDescription>
        </Alert>
      ))}
      {[...queryErrors, ...errors].map((message, index) => (
        <Alert
          key={`${message}-${index}`}
          variant="destructive"
          aria-label={message}
        >
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      ))}

      {dashboard.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : !selectedModule ? (
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          {t("dashboardModules.selectModule", {
            defaultValue: "Select a usage module",
          })}
        </div>
      ) : projection?.kind === "subscription" ? (
        <SubscriptionModulePage
          module={projection.module}
          providers={projection.providers}
          onOpenSettings={onOpenSettings}
          onRefreshQuota={(providerId) =>
            run(() => refreshQuota.mutateAsync(providerId))
          }
          onSyncSessions={sync}
          isRefreshingQuota={refreshQuota.isPending}
          isSyncingSessions={syncSession.isPending}
        />
      ) : projection?.kind === "api" ? (
        <ApiUsagePage
          projection={projection}
          startAt={range.startDate}
          endAt={range.endDate}
          onOpenSettings={onOpenSettings}
        />
      ) : null}
    </div>
  );
}
