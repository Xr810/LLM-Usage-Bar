import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { SegmentedControl } from "@/components/common/SegmentedControl";
import { UsageDateRangePicker } from "@/components/usage/UsageDateRangePicker";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import {
  useProviderUsageDashboard,
  useRefreshProviderQuota,
  useSyncProviderSessionUsage,
} from "@/lib/query/usageDashboard";
import { useProviderUsageActivity } from "@/lib/query/providerUsageActivity";
import { useSettingsQuery } from "@/lib/query";
import {
  resolveProviderActivityRange,
  resolveUsageRange,
} from "@/lib/usageRange";
import type { UsageRangeSelection } from "@/types/usage";
import type { AgentModuleView } from "@/types/usageDashboard";
import { ProviderUsagePage } from "./ProviderUsagePage";
import { projectProviderDashboard } from "./usageDashboardProjection";
import { DEFAULT_REMAINING_THRESHOLDS } from "./usagePresentation";

interface UsageDashboardPageProps {
  /** Compatibility-only; Provider monitoring intentionally ignores Agent selection. */
  selectedAgent?: AgentModuleView | null;
  onOpenSettings?: () => void;
}

type RangePreset = "today" | "7d" | "30d";

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
    preset: "30d",
  });
  const [warnings, setWarnings] = useState<string[]>([]);
  const [errors, setErrors] = useState<string[]>([]);
  const range = useMemo(
    () => resolveUsageRange(selection, rangeClockMs),
    [rangeClockMs, selection],
  );
  const activityRange = useMemo(
    () => resolveProviderActivityRange(rangeClockMs),
    [rangeClockMs],
  );
  const dashboard = useProviderUsageDashboard(range.startDate, range.endDate);
  const activity = useProviderUsageActivity(
    activityRange.startDate,
    activityRange.endDate,
  );
  const settings = useSettingsQuery();
  const refreshQuota = useRefreshProviderQuota();
  const syncSession = useSyncProviderSessionUsage();
  const projection = useMemo(
    () => (dashboard.data ? projectProviderDashboard(dashboard.data) : null),
    [dashboard.data],
  );
  const remainingThresholds = {
    warning:
      settings.data?.usageWarningRemainingPercent ??
      DEFAULT_REMAINING_THRESHOLDS.warning,
    critical:
      settings.data?.usageCriticalRemainingPercent ??
      DEFAULT_REMAINING_THRESHOLDS.critical,
  };

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
    ...(activity.error ? [errorText(activity.error)] : []),
    ...errors,
  ].filter((message, index, messages) => messages.indexOf(message) === index);

  const presetOptions = (["today", "7d", "30d"] as const).map((preset) => ({
    value: preset as RangePreset,
    label:
      preset === "today"
        ? t("usageDashboard.today", { defaultValue: "Today" })
        : preset === "7d"
          ? t("usageDashboard.sevenDays", { defaultValue: "7 days" })
          : t("usageDashboard.thirtyDays", { defaultValue: "30 days" }),
  }));

  return (
    <div className="space-y-5 pb-8">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold tracking-tight">
            {t("usageDashboard.providerMonitoring", {
              defaultValue: "Provider monitoring",
            })}
          </h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("usageDashboard.providerMonitoringDescription", {
              defaultValue:
                "Usage, cost and remaining quota are shown per Provider account.",
            })}
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <SegmentedControl
            options={presetOptions}
            value={
              selection.preset === "custom"
                ? ("" as RangePreset)
                : (selection.preset as RangePreset)
            }
            onChange={(preset) => setSelection({ preset })}
          />
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
        <div className="space-y-4" aria-hidden="true">
          <div className="grid gap-4 min-[900px]:grid-cols-[300px_minmax(0,1fr)]">
            <div className="h-[28rem] animate-pulse rounded-xl bg-muted/60" />
            <div className="space-y-4">
              <div className="h-52 animate-pulse rounded-xl bg-muted/60" />
              <div className="h-80 animate-pulse rounded-xl bg-muted/60" />
            </div>
          </div>
          <div className="h-16 animate-pulse rounded-xl bg-muted/60" />
        </div>
      ) : projection ? (
        <ProviderUsagePage
          projection={projection}
          startAt={range.startDate}
          endAt={range.endDate}
          activityBuckets={activity.data ?? []}
          activityStartAt={activityRange.startDate}
          activityEndAt={activityRange.endDate}
          isActivityLoading={activity.isLoading}
          onOpenSettings={onOpenSettings}
          onRefreshQuota={(providerId) =>
            run(() => refreshQuota.mutateAsync(providerId))
          }
          onSyncSessions={sync}
          isRefreshingQuota={refreshQuota.isPending}
          isSyncingSessions={syncSession.isPending}
          remainingThresholds={remainingThresholds}
        />
      ) : null}
    </div>
  );
}
