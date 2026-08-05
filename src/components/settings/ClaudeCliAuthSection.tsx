import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LogIn, LogOut, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { relativeTimeAgo } from "@/lib/relativeTime";
import {
  useClaudeCliAuthActions,
  useClaudeCliAuthStatus,
} from "@/lib/query/usageDashboard";

export function ClaudeCliAuthSection() {
  const { t, i18n } = useTranslation();
  const status = useClaudeCliAuthStatus();
  const actions = useClaudeCliAuthActions();
  const [failed, setFailed] = useState(false);

  const run = async (operation: () => Promise<unknown>) => {
    setFailed(false);
    try {
      await operation();
    } catch {
      setFailed(true);
    }
  };

  const data = status.data;
  const plan = data?.subscriptionType
    ? data.subscriptionType.charAt(0).toUpperCase() +
      data.subscriptionType.slice(1)
    : null;
  const connected = Boolean(data?.installed && data.authenticated);
  const detectedDetail = plan
    ? plan
    : data?.authMethod === "api_key"
      ? t("usageDashboard.claudeApiKey", { defaultValue: "API Key" })
      : data?.authMethod === "claude_account"
        ? t("usageDashboard.claudeAccount", {
            defaultValue: "Claude Account",
          })
        : t("usageDashboard.claudeAuthenticated", {
            defaultValue: "Authenticated",
          });

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3 rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15">
        <div className="flex min-w-0 items-center gap-2.5">
          <span
            aria-hidden="true"
            className={cn(
              "h-2 w-2 shrink-0 rounded-full",
              connected ? "bg-success" : "bg-muted-foreground/50",
            )}
          />
          <div className="min-w-0">
            <div className="truncate text-sm font-medium">
              {status.isLoading
                ? t("common.loading", { defaultValue: "Loading" })
                : !data?.installed
                  ? t("usageDashboard.claudeCliNotInstalled", {
                      defaultValue: "Claude CLI not installed",
                    })
                  : data.authenticated
                    ? t("usageDashboard.claudeCliDetected", {
                        defaultValue: `Claude CLI detected${detectedDetail ? ` · ${detectedDetail}` : ""}`,
                        detail: detectedDetail,
                      })
                    : t("usageDashboard.claudeCliDetectedSignedOut", {
                        defaultValue: "Claude CLI detected · Not signed in",
                      })}
            </div>
            {/* Quota does not come from the CLI, so this line used to read
                "Quota unavailable" unconditionally — under a card whose
                dashboard was showing that quota. It reports the local sample
                instead. */}
            <div
              className="truncate text-xs text-muted-foreground"
              title={
                data?.lastQuotaSampleAt
                  ? new Date(data.lastQuotaSampleAt * 1000).toLocaleString()
                  : undefined
              }
            >
              {data?.lastQuotaSampleAt
                ? t("usageDashboard.claudeQuotaSampledAt", {
                    value: relativeTimeAgo(
                      data.lastQuotaSampleAt * 1000,
                      i18n.resolvedLanguage ?? i18n.language,
                    ),
                    defaultValue: `Quota updated ${relativeTimeAgo(
                      data.lastQuotaSampleAt * 1000,
                      i18n.resolvedLanguage ?? i18n.language,
                    )}`,
                  })
                : t("usageDashboard.claudeQuotaNoSample", {
                    defaultValue: "No quota sample yet",
                  })}
            </div>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2 text-xs"
            disabled={actions.isPending || !data?.installed}
            aria-label={
              data?.authenticated
                ? t("usageDashboard.reconnectClaude", {
                    defaultValue: "Reconnect Claude",
                  })
                : t("usageDashboard.signInWithClaude", {
                    defaultValue: "Sign in with Claude",
                  })
            }
            onClick={() => void run(actions.startLogin)}
          >
            <LogIn className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
            {data?.authenticated
              ? t("usageDashboard.reconnect", { defaultValue: "Reconnect" })
              : t("usageDashboard.signIn", { defaultValue: "Sign in" })}
          </Button>
          {data?.authenticated ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={actions.isPending}
              aria-label={t("usageDashboard.disconnectClaude", {
                defaultValue: "Disconnect Claude",
              })}
              onClick={() => void run(actions.logout)}
            >
              <LogOut className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
              {t("usageDashboard.disconnect", { defaultValue: "Disconnect" })}
            </Button>
          ) : null}
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7"
            disabled={status.isLoading}
            aria-label={t("common.refresh", { defaultValue: "Refresh" })}
            onClick={() => void status.refetch()}
          >
            <RefreshCw
              className={cn("h-3.5 w-3.5", status.isFetching && "animate-spin")}
              aria-hidden="true"
            />
          </Button>
        </div>
      </div>

      {failed ? (
        <div role="alert" className="text-sm text-destructive">
          {t("usageDashboard.claudeAuthActionFailed", {
            defaultValue: "Claude authentication action failed.",
          })}
        </div>
      ) : null}
    </div>
  );
}
