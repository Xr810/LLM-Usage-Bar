import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SegmentedControl } from "@/components/common/SegmentedControl";
import { UsageDateRangePicker } from "@/components/usage/UsageDateRangePicker";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import {
  useAgentUsageBreakdown,
  useModelUsageDashboard,
  useProviderUsageDashboard,
  useRefreshProviderQuota,
  useSyncProviderSessionUsage,
} from "@/lib/query/usageDashboard";
import { useProviderUsageActivity } from "@/lib/query/providerUsageActivity";
import { useSettingsQuery } from "@/lib/query";
import {
  getUsageRangePresetLabel,
  resolveProviderActivityRange,
  resolveUsageRange,
} from "@/lib/usageRange";
import type { UsageRangeSelection } from "@/types/usage";
import type { AgentModuleView } from "@/types/usageDashboard";
import { ProviderUsagePage } from "./ProviderUsagePage";
import { ModelUsagePage } from "./ModelUsagePage";
import { AgentBreakdownPage } from "./AgentBreakdownPage";
import { projectProviderDashboard } from "./usageDashboardProjection";
import { DEFAULT_REMAINING_THRESHOLDS } from "./usagePresentation";

interface UsageDashboardPageProps {
  /** Compatibility-only; Provider monitoring intentionally ignores Agent selection. */
  selectedAgent?: AgentModuleView | null;
  onOpenSettings?: () => void;
}

type RangePreset = "today" | "7d" | "30d" | "1y";

/** How usage is classified. The time range is shared across all three. */
type UsageDimension = "provider" | "model" | "agent";

export function UsageDashboardPage({
  onOpenSettings,
}: UsageDashboardPageProps) {
  const { t, i18n } = useTranslation();
  const [rangeClockMs, setRangeClockMs] = useState(() => Date.now());
  const advanceRangeClock = useCallback(() => setRangeClockMs(Date.now()), []);
  useUsageEventBridge(advanceRangeClock);
  useEffect(() => {
    const interval = globalThis.setInterval(advanceRangeClock, 60_000);
    return () => globalThis.clearInterval(interval);
  }, [advanceRangeClock]);

  const [dimension, setDimension] = useState<UsageDimension>("provider");
  const [selection, setSelection] = useState<UsageRangeSelection>({
    preset: "30d",
  });
  const [warnings, setWarnings] = useState<string[]>([]);
  const [errors, setErrors] = useState<string[]>([]);
  const range = useMemo(
    () => resolveUsageRange(selection, rangeClockMs),
    [rangeClockMs, selection],
  );
  const rangeLabel = useMemo(() => {
    if (selection.preset !== "custom") {
      return getUsageRangePresetLabel(selection.preset, t);
    }
    const locale = i18n.resolvedLanguage || i18n.language || "en";
    const formatter = new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
    const start = new Date(range.startDate * 1_000);
    const end = new Date(range.endDate * 1_000);
    return `${formatter.format(start)} – ${formatter.format(end)}`;
  }, [i18n.language, i18n.resolvedLanguage, range, selection.preset, t]);
  const activityRange = useMemo(
    () => resolveProviderActivityRange(rangeClockMs),
    [rangeClockMs],
  );
  const dashboard = useProviderUsageDashboard(range.startDate, range.endDate);
  const modelDashboard = useModelUsageDashboard(
    range.startDate,
    range.endDate,
    dimension === "model",
  );
  const agentBreakdown = useAgentUsageBreakdown(
    range.startDate,
    range.endDate,
    dimension === "agent",
  );
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
    ...(dimension === "provider" && dashboard.error
      ? [errorText(dashboard.error)]
      : []),
    ...(dimension === "provider" && activity.error
      ? [errorText(activity.error)]
      : []),
    ...(dimension === "model" && modelDashboard.error
      ? [errorText(modelDashboard.error)]
      : []),
    ...(dimension === "agent" && agentBreakdown.error
      ? [errorText(agentBreakdown.error)]
      : []),
    ...errors,
  ].filter((message, index, messages) => messages.indexOf(message) === index);

  const presetOptions = (["today", "7d", "30d", "1y"] as const).map(
    (preset) => ({
      value: preset as RangePreset,
      label:
        preset === "today"
          ? t("usageDashboard.today", { defaultValue: "Today" })
          : preset === "7d"
            ? t("usageDashboard.sevenDays", { defaultValue: "7 days" })
            : preset === "30d"
              ? t("usageDashboard.thirtyDays", { defaultValue: "30 days" })
              : t("usageDashboard.oneYear", { defaultValue: "1 year" }),
    }),
  );

  const rangeControls = (
    <div
      className="flex flex-wrap items-center justify-end gap-2"
      role="group"
      aria-label={t("usage.timeRange", { defaultValue: "Time range" })}
    >
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
  );

  const heading =
    dimension === "model"
      ? {
          title: t("usageBreakdown.modelTitle", { defaultValue: "By model" }),
          description: t("usageBreakdown.modelDescription", {
            defaultValue:
              "Roll usage up to a subscription or API plan, then drill into the models it served.",
          }),
        }
      : dimension === "agent"
        ? {
            title: t("usageBreakdown.agentTitle", { defaultValue: "By agent" }),
            description: t("usageBreakdown.agentDescription", {
              defaultValue:
                "What each Agent spent in this range, and which accounts and models it used.",
            }),
          }
        : {
            title: t("usageDashboard.providerMonitoring", {
              defaultValue: "Provider monitoring",
            }),
            description: t("usageDashboard.providerMonitoringDescription", {
              defaultValue:
                "Usage, cost and remaining quota are shown per Provider account.",
            }),
          };

  const dimensionOptions: { value: UsageDimension; label: string }[] = [
    {
      value: "provider",
      label: t("usageBreakdown.dimensionProvider", {
        defaultValue: "Providers",
      }),
    },
    {
      value: "model",
      label: t("usageBreakdown.dimensionModel", { defaultValue: "Models" }),
    },
    {
      value: "agent",
      label: t("usageBreakdown.dimensionAgent", { defaultValue: "Agents" }),
    },
  ];

  const breakdownSkeleton = (
    <div className="space-y-4" aria-hidden="true">
      <div className="h-20 animate-pulse rounded-xl bg-muted/60" />
      <div className="h-96 animate-pulse rounded-xl bg-muted/60" />
    </div>
  );

  return (
    <div className="usage-dashboard-shell space-y-5 pb-8">
      <div className="usage-dashboard-toolbar flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold tracking-tight">
            {heading.title}
          </h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {heading.description}
          </p>
        </div>
        <Tabs
          value={dimension}
          onValueChange={(value) => setDimension(value as UsageDimension)}
        >
          <TabsList
            aria-label={t("usageBreakdown.dimensionNavigation", {
              defaultValue: "Usage breakdown",
            })}
          >
            {dimensionOptions.map((option) => (
              <TabsTrigger key={option.value} value={option.value}>
                {option.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>

      {[
        ...(dimension === "provider" ? (dashboard.data?.warnings ?? []) : []),
        ...warnings,
      ].map((warning) => (
        <Alert key={warning}>
          <AlertDescription>{warning}</AlertDescription>
        </Alert>
      ))}
      {renderedErrors.map((message) => (
        <Alert key={message} variant="destructive" aria-label={message}>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      ))}

      {dimension === "provider" ? (
        dashboard.isLoading ? (
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
            rangeLabel={rangeLabel}
            rangeControls={rangeControls}
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
        ) : null
      ) : dimension === "model" ? (
        modelDashboard.data ? (
          <ModelUsagePage
            dashboard={modelDashboard.data}
            rangeLabel={rangeLabel}
            rangeControls={rangeControls}
          />
        ) : (
          breakdownSkeleton
        )
      ) : agentBreakdown.data ? (
        <AgentBreakdownPage
          breakdown={agentBreakdown.data}
          rangeLabel={rangeLabel}
          rangeControls={rangeControls}
        />
      ) : (
        breakdownSkeleton
      )}
    </div>
  );
}
