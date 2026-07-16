import { useTranslation } from "react-i18next";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  clampPercentForProgress,
  formatPercent,
  formatResetTime,
  hasUsablePercent,
  providerIconName,
  quotaUnavailableReasonLabel,
  type TrayProviderRow,
} from "./trayUsagePresentation";
import { TrayUsageProgress, TrayUsageStatusBadge } from "./TrayUsageProgress";

export function TraySubscriptionSection({
  rows,
  now,
  locale,
  showAgentName,
}: {
  rows: TrayProviderRow[];
  now: Date;
  locale: string;
  showAgentName: boolean;
}) {
  const { t } = useTranslation();
  if (rows.length === 0) return null;

  return (
    <section aria-labelledby="tray-subscription-heading">
      <h2
        id="tray-subscription-heading"
        className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
      >
        {t("trayUsage.subscription", { defaultValue: "Subscription" })}
      </h2>
      <div className="divide-y divide-border rounded-lg border border-border">
        {rows.map(({ agentModuleId, agentName, provider }) => {
          if (!provider.subscription) return null;
          const planText = provider.subscription.planLabel
            ? t("trayUsage.plan", {
                plan: provider.subscription.planLabel,
                defaultValue: "Plan: {{plan}}",
              })
            : null;
          const subtitle = showAgentName
            ? [planText, agentName].filter(Boolean).join(" · ")
            : planText;
          return (
            <article
              key={`${agentModuleId}:${provider.providerId}`}
              className="tray-usage-provider-row space-y-3 px-3 py-3"
            >
              <div className="flex min-w-0 items-start justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  <ProviderIcon
                    icon={providerIconName(provider.systemPresetKey)}
                    name={provider.providerName}
                    size={26}
                    className="rounded-md"
                  />
                  <div className="min-w-0">
                    <h3
                      className="truncate text-sm font-semibold"
                      title={provider.providerName}
                    >
                      {provider.providerName}
                    </h3>
                    {subtitle ? (
                      <p className="truncate text-[11px] text-muted-foreground">
                        {subtitle}
                      </p>
                    ) : null}
                  </div>
                </div>
                <TrayUsageStatusBadge status={provider.status} t={t} />
              </div>

              <div className="space-y-3">
                {provider.subscription.windows.map((window) => {
                  const allowanceLabel =
                    window.kind === "five_hour"
                      ? t("trayUsage.fiveHourAllowance", {
                          defaultValue: "5-hour allowance",
                        })
                      : t("trayUsage.sevenDayAllowance", {
                          defaultValue: "7-day allowance",
                        });
                  const reset = formatResetTime(
                    window.resetsAt,
                    now,
                    locale,
                    t("trayUsage.pendingRefresh", {
                      defaultValue: "Pending refresh",
                    }),
                  );
                  const progressLabel = showAgentName
                    ? t("trayUsage.providerAllowanceForAgent", {
                        allowance: allowanceLabel,
                        provider: provider.providerName,
                        agent: agentName,
                        defaultValue:
                          "{{allowance}} for {{provider}} · {{agent}}",
                      })
                    : t("trayUsage.providerAllowance", {
                        allowance: allowanceLabel,
                        provider: provider.providerName,
                        defaultValue: "{{allowance}} for {{provider}}",
                      });
                  const unavailableReason = quotaUnavailableReasonLabel(
                    window.unavailableReason,
                    t,
                  );
                  const resetLabel = reset.pending
                    ? reset.text
                    : t("trayUsage.resets", {
                        time: reset.text,
                        defaultValue: "Resets {{time}}",
                      });
                  return (
                    <div key={window.kind} className="space-y-1.5">
                      <div className="flex items-baseline justify-between gap-3 text-xs">
                        <span className="font-medium">{allowanceLabel}</span>
                        <span className="font-semibold tabular-nums">
                          {t("trayUsage.remaining", {
                            percent: formatPercent(window.remainingPercent),
                            defaultValue: "{{percent}} remaining",
                          })}
                        </span>
                      </div>
                      {hasUsablePercent(window.usedPercent) ? (
                        <TrayUsageProgress
                          label={progressLabel}
                          value={clampPercentForProgress(window.usedPercent)}
                          status={window.status}
                        />
                      ) : null}
                      <div className="flex items-center justify-between gap-3 text-[11px] text-muted-foreground">
                        <span className="min-w-0">
                          <span className="block">{resetLabel}</span>
                          {unavailableReason ? (
                            <span className="block text-foreground">
                              {unavailableReason}
                            </span>
                          ) : null}
                        </span>
                        <TrayUsageStatusBadge status={window.status} t={t} />
                      </div>
                    </div>
                  );
                })}
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
