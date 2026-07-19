import { useTranslation } from "react-i18next";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ManualResetCredits } from "@/components/ManualResetCredits";
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
import { TrayProviderRecentUsage } from "./TrayProviderRecentUsage";

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
        className="mb-1.5 px-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground"
      >
        {t("trayUsage.subscription", { defaultValue: "Subscription" })}
      </h2>
      <div className="space-y-2">
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
              className="tray-usage-provider-row space-y-2.5 rounded-xl border border-border/60 bg-card"
            >
              <div className="flex min-w-0 items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <ProviderIcon
                    icon={providerIconName(provider.systemPresetKey)}
                    name={provider.providerName}
                    size={22}
                    className="rounded-md"
                  />
                  <div className="min-w-0">
                    <h3
                      className="truncate text-[13px] font-semibold leading-tight"
                      title={provider.providerName}
                    >
                      {provider.providerName}
                    </h3>
                    {subtitle ? (
                      <p className="truncate text-[11px] leading-tight text-muted-foreground">
                        {subtitle}
                      </p>
                    ) : null}
                  </div>
                </div>
                <TrayUsageStatusBadge status={provider.status} t={t} />
              </div>

              <div className="space-y-2.5">
                {provider.subscription.windows.map((window) => {
                  const allowanceLabel =
                    window.kind === "five_hour"
                      ? t("trayUsage.fiveHourAllowance", {
                          defaultValue: "5-hour allowance",
                        })
                      : t("trayUsage.sevenDayAllowance", {
                          defaultValue: "Weekly allowance",
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
                    <div key={window.kind} className="space-y-1">
                      <div className="flex items-baseline justify-between gap-3 text-xs">
                        <span className="text-muted-foreground">
                          {allowanceLabel}
                        </span>
                        <span className="font-semibold tabular-nums">
                          {t("trayUsage.remaining", {
                            percent: formatPercent(window.remainingPercent),
                            defaultValue: "{{percent}} remaining",
                          })}
                        </span>
                      </div>
                      {hasUsablePercent(window.remainingPercent) ? (
                        <TrayUsageProgress
                          label={progressLabel}
                          value={clampPercentForProgress(
                            window.remainingPercent,
                          )}
                          status={window.status}
                        />
                      ) : null}
                      <div className="text-[11px] leading-tight text-muted-foreground">
                        <span>{resetLabel}</span>
                        {unavailableReason ? (
                          <span className="text-foreground">
                            {" · "}
                            {unavailableReason}
                          </span>
                        ) : null}
                      </div>
                    </div>
                  );
                })}
              </div>

              <ManualResetCredits
                availableCount={
                  provider.subscription.manualResetsRemaining ?? null
                }
                credits={provider.subscription.manualResetCredits ?? []}
                compact
              />

              <TrayProviderRecentUsage
                usage={provider.recentUsage}
                locale={locale}
              />
            </article>
          );
        })}
      </div>
    </section>
  );
}
