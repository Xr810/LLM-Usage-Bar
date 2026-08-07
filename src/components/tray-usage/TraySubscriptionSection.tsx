import { useTranslation } from "react-i18next";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ManualResetCredits } from "@/components/ManualResetCredits";
import {
  clampPercentForProgress,
  formatPercent,
  formatResetTime,
  hasUsablePercent,
  providerDisplayName,
  providerIconName,
  quotaUnavailableReasonLabel,
  rhythmExplanation,
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
          // The tier belongs in the name — "ChatGPT Pro" is what the account
          // is called. Filed underneath as "Plan: Pro" it read as a separate
          // fact about a Provider whose name looked incomplete.
          const displayName = providerDisplayName(
            provider.providerName,
            provider.subscription.planLabel,
          );
          const subtitle = showAgentName ? agentName : null;
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
                  {/* Plan sits inline with the name rather than on its own
                      line — in a 520px popover every saved row is another
                      account visible without scrolling. */}
                  <h3
                    className="flex min-w-0 items-baseline gap-1.5 text-[13px] font-semibold leading-tight"
                    title={displayName}
                  >
                    <span className="min-w-0 truncate">{displayName}</span>
                    {/* The account name identifies the row, the plan only
                        qualifies it — so the plan gives up width first. */}
                    {subtitle ? (
                      <span className="min-w-0 shrink-[999] truncate text-[11px] font-normal text-muted-foreground">
                        {subtitle}
                      </span>
                    ) : null}
                  </h3>
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
                  const rhythmNote = rhythmExplanation(window, t);
                  const resetLabel = reset.pending
                    ? reset.text
                    : t("trayUsage.resets", {
                        time: reset.text,
                        defaultValue: "Resets {{time}}",
                      });
                  return (
                    // Two lines per window instead of three: the reset time
                    // rides beside the window label, leaving label+value, bar.
                    <div key={window.kind} className="space-y-1">
                      <div className="flex items-baseline justify-between gap-3 text-xs">
                        <span className="flex min-w-0 items-baseline gap-1.5">
                          <span className="shrink-0 text-muted-foreground">
                            {allowanceLabel}
                          </span>
                          <span className="truncate text-[11px] text-muted-foreground/80">
                            {resetLabel}
                            {unavailableReason ? (
                              <span className="text-foreground">
                                {" · "}
                                {unavailableReason}
                              </span>
                            ) : null}
                          </span>
                        </span>
                        <span className="shrink-0 font-semibold tabular-nums">
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
                      {rhythmNote ? (
                        <p className="text-[11px] leading-tight text-muted-foreground/80">
                          {rhythmNote}
                        </p>
                      ) : null}
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
