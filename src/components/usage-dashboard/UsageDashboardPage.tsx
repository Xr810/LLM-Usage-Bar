import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { UsageDateRangePicker } from "@/components/usage/UsageDateRangePicker";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import {
  useProviderUsageDashboard,
  useRefreshProviderQuota,
  useSyncProviderSessionUsage,
} from "@/lib/query/usageDashboard";
import { resolveUsageRange } from "@/lib/usageRange";
import type { UsageRangeSelection } from "@/types/usage";
import type { AgentModuleView } from "@/types/usageDashboard";
import { ProviderUsagePage } from "./ProviderUsagePage";
import { projectProviderDashboard } from "./usageDashboardProjection";

interface UsageDashboardPageProps {
  /** Compatibility-only; Provider monitoring intentionally ignores Agent selection. */
  selectedAgent?: AgentModuleView | null;
  onOpenSettings?: () => void;
}

export function UsageDashboardPage({
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
  const dashboard = useProviderUsageDashboard(
    range.startDate,
    range.endDate,
  );
  const refreshQuota = useRefreshProviderQuota();
  const syncSession = useSyncProviderSessionUsage();
  const projection = useMemo(
    () =>
      dashboard.data ? projectProviderDashboard(dashboard.data) : null,
    [dashboard.data],
  );

  const errorText = (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause);
  const run = async (operation: () => Promise<unknown>) => {
    try {
      await operation();
      setErrors([]);
    } catch (cause) {
      setErrors([errorText(cause)]);
    }
  };
  const sync = async (providerId: string) => {
    try {
      const result = await syncSession.mutateAsync(providerId);
      setWarnings(result.warnings ?? []);
      setErrors(result.errors ?? []);
    } catch (cause) {
      setWarnings([]);
      setErrors([errorText(cause)]);
    }
  };

  const renderedErrors = [
    ...(dashboard.error ? [errorText(dashboard.error)] : []),
    ...errors,
  ].filter((message, index, messages) => messages.indexOf(message) === index);

  return (
    <div className="space-y-4 pb-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold">
            {t("usageDashboard.providerMonitoring", {
              defaultValue: "Provider monitoring",
            })}
          </h2>
          <p className="text-xs text-muted-foreground">
            {t("usageDashboard.providerMonitoringDescription", {
              defaultValue:
                "Usage, cost and remaining quota are shown per Provider account.",
            })}
          </p>
        </div>
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
      </div>

      {[...(dashboard.data?.warnings ?? []), ...warnings].map((warning) => (
        <Alert key={warning}>
          <AlertDescription>{warning}</AlertDescription>
        </Alert>
      ))}
      {renderedErrors.map((message) => (
        <Alert key={message} variant="destructive" aria-label={message}>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      ))}

      {dashboard.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : projection ? (
        <ProviderUsagePage
          projection={projection}
          startAt={range.startDate}
          endAt={range.endDate}
          onOpenSettings={onOpenSettings}
          onRefreshQuota={(providerId) =>
            run(() => refreshQuota.mutateAsync(providerId))
          }
          onSyncSessions={sync}
          isRefreshingQuota={refreshQuota.isPending}
          isSyncingSessions={syncSession.isPending}
        />
      ) : null}
    </div>
  );
}
