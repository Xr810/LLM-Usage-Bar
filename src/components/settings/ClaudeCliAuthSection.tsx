import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  useClaudeCliAuthActions,
  useClaudeCliAuthStatus,
} from "@/lib/query/usageDashboard";

export function ClaudeCliAuthSection() {
  const { t } = useTranslation();
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

  return (
    <div className="space-y-3 rounded-md border bg-muted/20 p-3">
      <div className="space-y-1">
        <div className="text-sm font-medium">
          {status.isLoading
            ? t("common.loading", { defaultValue: "Loading" })
            : !data?.installed
              ? t("usageDashboard.claudeCliNotInstalled", {
                  defaultValue: "Claude CLI not installed",
                })
              : data.authenticated
                ? t("usageDashboard.claudeConnected", {
                    defaultValue: `Connected${plan ? ` · ${plan}` : ""}`,
                    plan,
                  })
                : t("usageDashboard.claudeDisconnected", {
                    defaultValue: "Not connected",
                  })}
        </div>
        <div className="text-xs text-muted-foreground">
          {t("usageDashboard.claudeQuotaUnavailable", {
            defaultValue: "Quota unavailable",
          })}
        </div>
      </div>

      {failed ? (
        <div role="alert" className="text-sm text-destructive">
          {t("usageDashboard.claudeAuthActionFailed", {
            defaultValue: "Claude authentication action failed.",
          })}
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
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
          {data?.authenticated
            ? t("usageDashboard.reconnect", { defaultValue: "Reconnect" })
            : t("usageDashboard.signIn", { defaultValue: "Sign in" })}
        </Button>
        {data?.authenticated ? (
          <Button
            size="sm"
            variant="outline"
            disabled={actions.isPending}
            aria-label={t("usageDashboard.disconnectClaude", {
              defaultValue: "Disconnect Claude",
            })}
            onClick={() => void run(actions.logout)}
          >
            {t("usageDashboard.disconnect", { defaultValue: "Disconnect" })}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          disabled={status.isLoading}
          onClick={() => void status.refetch()}
        >
          {t("common.refresh", { defaultValue: "Refresh" })}
        </Button>
      </div>
    </div>
  );
}
