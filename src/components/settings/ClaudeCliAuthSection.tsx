import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyRound, LogIn, LogOut, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { relativeTimeAgo } from "@/lib/relativeTime";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ToggleRow } from "@/components/ui/toggle-row";
import { useSaveSettingsMutation, useSettingsQuery } from "@/lib/query";
import {
  useClaudeCliAuthActions,
  useClaudeCliAuthStatus,
} from "@/lib/query/usageDashboard";

export function ClaudeCliAuthSection() {
  const { t, i18n } = useTranslation();
  const status = useClaudeCliAuthStatus();
  const actions = useClaudeCliAuthActions();
  const settingsQuery = useSettingsQuery();
  const saveSettings = useSaveSettingsMutation();
  const [failed, setFailed] = useState(false);
  const [consentDialogOpen, setConsentDialogOpen] = useState(false);

  const run = async (operation: () => Promise<unknown>) => {
    setFailed(false);
    try {
      await operation();
    } catch {
      setFailed(true);
    }
  };

  const oauthQuotaEnabled = settingsQuery.data?.claudeOauthQuotaEnabled ?? false;

  const saveConsent = (value: boolean) => {
    const current = settingsQuery.data;
    if (!current) return;
    saveSettings.mutate({ ...current, claudeOauthQuotaEnabled: value });
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

      <ToggleRow
        icon={<KeyRound className="h-4 w-4" aria-hidden="true" />}
        title={t("usageDashboard.claudeOauthQuotaToggleTitle", {
          defaultValue: "Query official quota with Claude Code credentials",
        })}
        description={t("usageDashboard.claudeOauthQuotaToggleDescription", {
          defaultValue:
            "Reads Claude Code's login stored in the macOS keychain to fetch exact usage windows from Anthropic. Off by default; you can revoke it here at any time.",
        })}
        checked={oauthQuotaEnabled}
        disabled={saveSettings.isPending}
        onCheckedChange={(value) => {
          if (value) {
            setConsentDialogOpen(true);
          } else {
            saveConsent(false);
          }
        }}
      />

      <ConfirmDialog
        isOpen={consentDialogOpen}
        variant="info"
        title={t("usageDashboard.claudeOauthConsentTitle", {
          defaultValue: "Allow reading Claude Code's login?",
        })}
        message={t("usageDashboard.claudeOauthConsentMessage", {
          defaultValue:
            "Enabling this lets LLM Usage Bar read the login that Claude Code stores in the macOS keychain, and use it to query Anthropic's official quota endpoint.\n\nIt reads nothing else, the credential never leaves this device, and you can turn this off here at any time. macOS may ask you to confirm access when the quota is refreshed.",
        })}
        checkboxLabel={t("usageDashboard.claudeOauthConsentCheckbox", {
          defaultValue:
            "I understand that Claude Code's keychain login will be read to query the official quota.",
        })}
        confirmText={t("usageDashboard.claudeOauthConsentConfirm", {
          defaultValue: "Allow",
        })}
        onConfirm={(checkboxChecked) => {
          setConsentDialogOpen(false);
          if (checkboxChecked) {
            saveConsent(true);
          }
        }}
        onCancel={() => setConsentDialogOpen(false)}
      />
    </div>
  );
}
