import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { relativeTimeAgo } from "@/lib/relativeTime";
import { useSystemProviderCredentialActions } from "@/lib/query/usageDashboard";
import type { ProviderApiKeyView } from "@/types/usageDashboard";
import {
  credentialErrorCodeOf,
  credentialErrorCopy,
} from "./providerCredentialErrors";
import { formatUsd, spendBarClass, spentRatio } from "./providerSpendFormat";

interface ProviderApiKeyRowProps {
  apiKey: ProviderApiKeyView;
  /** OpenRouter can verify a key against /key; NVIDIA exposes no equivalent. */
  canTestConnection: boolean;
  supportsKeyUsage: boolean;
  onRequestDelete: (apiKey: ProviderApiKeyView) => void;
}

export function ProviderApiKeyRow({
  apiKey,
  canTestConnection,
  supportsKeyUsage,
  onRequestDelete,
}: ProviderApiKeyRowProps) {
  const { t, i18n } = useTranslation();
  const credentials = useSystemProviderCredentialActions();
  const [expanded, setExpanded] = useState(false);
  const [editingLabel, setEditingLabel] = useState<string | null>(null);
  const [secret, setSecret] = useState("");
  const [replacing, setReplacing] = useState(false);
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<
    "success" | "failed" | null
  >(null);

  const locale = i18n.resolvedLanguage ?? i18n.language;
  const configured = apiKey.credentialStatus === "configured";
  const unreadable = apiKey.credentialStatus === "unavailable";
  const showSecretForm = replacing || !configured;
  const usage = apiKey.keyUsage;

  const run = async (operation: () => Promise<unknown>) => {
    setErrorCode(null);
    try {
      return await operation();
    } catch (cause) {
      setErrorCode(credentialErrorCodeOf(cause));
      return undefined;
    }
  };

  const submitSecret = async () => {
    const transient = secret.trim();
    if (!transient) return;
    setSecret("");
    setConnectionStatus(null);
    await run(async () => {
      const updated = configured
        ? await credentials.replaceApiKey(
            apiKey.id,
            apiKey.credentialVersion,
            transient,
          )
        : await credentials.setApiKey(
            apiKey.id,
            apiKey.credentialVersion,
            transient,
          );
      setReplacing(false);
      if (canTestConnection) {
        const result = await credentials.testConnection(
          apiKey.id,
          updated.credentialVersion,
        );
        setConnectionStatus(result.status);
      }
      if (supportsKeyUsage) {
        await credentials.refreshKeyUsage(apiKey.id, updated.credentialVersion);
      }
    });
  };

  const commitLabel = async () => {
    const next = (editingLabel ?? "").trim();
    setEditingLabel(null);
    if (!next || next === apiKey.label) return;
    await run(() => credentials.renameKey(apiKey.id, next));
  };

  const shownTestStatus = connectionStatus ?? apiKey.lastConnectionTestStatus;
  const testedAtMs = apiKey.lastConnectionTestAt
    ? apiKey.lastConnectionTestAt * 1000
    : null;
  const testResultLabel =
    shownTestStatus === "success"
      ? t("usageDashboard.connectionSucceeded", {
          defaultValue: "Connection succeeded",
        })
      : shownTestStatus === "failed"
        ? t("usageDashboard.connectionFailed", {
            defaultValue: "Connection failed",
          })
        : null;
  const detail = !configured
    ? null
    : testResultLabel === null
      ? t("usageDashboard.apiKeyNeverTested", {
          defaultValue: "Not tested yet",
        })
      : testedAtMs !== null && connectionStatus === null
        ? t("usageDashboard.apiKeyLastTest", {
            result: testResultLabel,
            value: relativeTimeAgo(testedAtMs, locale),
            defaultValue: `Last test: ${testResultLabel} · ${relativeTimeAgo(testedAtMs, locale)}`,
          })
        : testResultLabel;

  const ratio = spentRatio(usage?.limitUsd, usage?.limitRemainingUsd);
  const error = credentialErrorCopy(errorCode);

  return (
    <div className="rounded-lg border border-border/60 bg-muted/20 dark:bg-muted/10">
      <div className="flex items-center gap-2 px-2.5 py-2">
        <Button
          size="icon"
          variant="ghost"
          className="size-6 shrink-0"
          aria-label={t(
            expanded
              ? "usageDashboard.collapseKey"
              : "usageDashboard.expandKey",
            {
              name: apiKey.label,
              defaultValue: `${expanded ? "Collapse" : "Expand"} ${apiKey.label}`,
            },
          )}
          onClick={() => setExpanded((open) => !open)}
        >
          <ChevronRight
            className={cn(
              "size-3.5 transition-transform",
              expanded && "rotate-90",
            )}
            aria-hidden="true"
          />
        </Button>

        <span
          aria-hidden="true"
          className={cn(
            "size-2 shrink-0 rounded-full",
            unreadable
              ? "bg-destructive"
              : !configured
                ? "bg-muted-foreground/40"
                : shownTestStatus === "success"
                  ? "bg-success"
                  : shownTestStatus === "failed"
                    ? "bg-warning"
                    : "bg-muted-foreground/50",
          )}
        />

        {editingLabel === null ? (
          <button
            type="button"
            className="min-w-0 truncate rounded px-1 text-sm font-medium hover:bg-muted/60 hover:underline"
            title={t("usageDashboard.renameKeyHint", {
              name: apiKey.label,
              defaultValue: `Rename ${apiKey.label}`,
            })}
            onClick={() => setEditingLabel(apiKey.label)}
          >
            {apiKey.label}
          </button>
        ) : (
          <Input
            autoFocus
            className="h-7 max-w-[12rem] text-sm"
            aria-label={t("usageDashboard.keyLabel", {
              defaultValue: "Key name",
            })}
            value={editingLabel}
            onChange={(event) => setEditingLabel(event.target.value)}
            onBlur={() => void commitLabel()}
            onKeyDown={(event) => {
              if (event.key === "Enter") void commitLabel();
              if (event.key === "Escape") setEditingLabel(null);
            }}
          />
        )}

        <div className="ml-auto flex shrink-0 items-center gap-1">
          {/* Fixed width so the action buttons stay in one column whether or not
              a key has a figure yet — a ragged edge reads as a layout bug. */}
          <span className="mr-1 min-w-[5rem] text-right text-sm font-medium tabular-nums">
            {formatUsd(usage?.usageTotalUsd, locale) ?? ""}
          </span>
          {configured && canTestConnection ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={credentials.isPending}
              onClick={() =>
                void run(async () => {
                  setConnectionStatus(null);
                  const result = await credentials.testConnection(
                    apiKey.id,
                    apiKey.credentialVersion,
                  );
                  setConnectionStatus(result.status);
                })
              }
            >
              {t("usageDashboard.testConnection", {
                defaultValue: "Test connection",
              })}
            </Button>
          ) : null}
          {configured ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={credentials.isPending}
              onClick={() => {
                setErrorCode(null);
                setReplacing((open) => !open);
              }}
            >
              {replacing
                ? t("common.cancel", { defaultValue: "Cancel" })
                : t("usageDashboard.replaceApiKey", {
                    defaultValue: "Replace",
                  })}
            </Button>
          ) : null}
          <Button
            size="icon"
            variant="ghost"
            className="size-7 hover:bg-destructive/10 hover:text-destructive"
            disabled={credentials.isPending}
            aria-label={t("usageDashboard.deleteKey", {
              name: apiKey.label,
              defaultValue: `Delete ${apiKey.label}`,
            })}
            onClick={() => onRequestDelete(apiKey)}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
          </Button>
        </div>
      </div>

      {detail || unreadable ? (
        <div className="px-2.5 pb-2 pl-[3.25rem] text-xs text-muted-foreground">
          {unreadable
            ? t("usageDashboard.apiKeyUnreadableHint", {
                defaultValue: "Enter it again to restore access.",
              })
            : detail}
        </div>
      ) : null}

      {showSecretForm ? (
        <form
          className="flex items-center gap-2 px-2.5 pb-2.5 pl-[3.25rem]"
          onSubmit={(event) => {
            event.preventDefault();
            void submitSecret();
          }}
        >
          <Input
            type="password"
            autoComplete="off"
            className="h-8"
            aria-label={t("usageDashboard.apiKey", { defaultValue: "API key" })}
            placeholder={t("usageDashboard.apiKeyPlaceholder", {
              defaultValue: "Enter API key",
            })}
            value={secret}
            onChange={(event) => setSecret(event.target.value)}
          />
          <Button
            type="submit"
            size="sm"
            className="h-8 shrink-0"
            disabled={credentials.isPending || secret.trim().length === 0}
          >
            {t("usageDashboard.verifyApiKey", { defaultValue: "Verify" })}
          </Button>
        </form>
      ) : null}

      {expanded && configured ? (
        <div className="space-y-2 border-t border-border/50 px-2.5 py-2.5 pl-[3.25rem]">
          {usage ? (
            <>
              <div className="flex flex-wrap gap-x-6 gap-y-1.5">
                <KeyStat
                  label={t("usageDashboard.keySpendToday", {
                    defaultValue: "Today (UTC)",
                  })}
                  value={formatUsd(usage.usageDailyUsd, locale)}
                />
                <KeyStat
                  label={t("usageDashboard.keySpendMonth", {
                    defaultValue: "This month (UTC)",
                  })}
                  value={formatUsd(usage.usageMonthlyUsd, locale)}
                />
              </div>
              {ratio !== null ? (
                <div className="space-y-1">
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted-foreground/15">
                    <div
                      className={cn(
                        "h-full rounded-full transition-[width]",
                        spendBarClass(ratio),
                      )}
                      style={{ width: `${Math.round(ratio * 100)}%` }}
                    />
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {t("usageDashboard.keySpendRemaining", {
                      remaining: formatUsd(usage.limitRemainingUsd, locale),
                      limit: formatUsd(usage.limitUsd, locale),
                      defaultValue: `${formatUsd(usage.limitRemainingUsd, locale)} left of ${formatUsd(usage.limitUsd, locale)}`,
                    })}
                  </div>
                </div>
              ) : (
                <div className="text-xs text-muted-foreground">
                  {t("usageDashboard.keySpendNoLimit", {
                    defaultValue: "No spending cap set on this key",
                  })}
                </div>
              )}
              <div
                className="text-xs text-muted-foreground"
                title={new Date(usage.fetchedAt * 1000).toLocaleString()}
              >
                {t("usageDashboard.keySpendFetchedAt", {
                  value: relativeTimeAgo(usage.fetchedAt * 1000, locale),
                  defaultValue: `Updated ${relativeTimeAgo(usage.fetchedAt * 1000, locale)}`,
                })}
              </div>
              {usage.stale ? (
                <div className="rounded-md border border-warning/20 bg-warning/10 px-2 py-1 text-xs text-warning">
                  {t("usageDashboard.keySpendStale", {
                    defaultValue:
                      "These figures belong to a previous key. Refresh to update them.",
                  })}
                </div>
              ) : null}
            </>
          ) : (
            <div className="text-xs text-muted-foreground">
              {t("usageDashboard.keySpendNeverFetched", {
                defaultValue:
                  "Not fetched yet. This reads the Provider's own figures for the saved key.",
              })}
            </div>
          )}
          {supportsKeyUsage ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-7 gap-1.5 px-2 text-xs"
              disabled={credentials.isPending}
              onClick={() =>
                void run(() =>
                  credentials.refreshKeyUsage(
                    apiKey.id,
                    apiKey.credentialVersion,
                  ),
                )
              }
            >
              <RefreshCw
                className={cn(
                  "size-3.5",
                  credentials.isPending && "animate-spin",
                )}
                aria-hidden="true"
              />
              {usage
                ? t("common.refresh", { defaultValue: "Refresh" })
                : t("usageDashboard.checkKeySpend", {
                    defaultValue: "Check spend",
                  })}
            </Button>
          ) : null}
        </div>
      ) : null}

      {error ? (
        <div
          role="alert"
          className="px-2.5 pb-2.5 pl-[3.25rem] text-sm text-destructive"
        >
          {t(error.key, { defaultValue: error.fallback })}
        </div>
      ) : null}
    </div>
  );
}

function KeyStat({ label, value }: { label: string; value: string | null }) {
  return (
    <div>
      <div className="text-sm font-medium tabular-nums">{value ?? "—"}</div>
      <div className="mt-0.5 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}
