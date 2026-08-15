import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { relativeTimeAgo } from "@/lib/relativeTime";
import { useSystemProviderCredentialActions } from "@/lib/query/usageDashboard";
import type {
  ProviderApiKeyView,
  UsageProviderView,
} from "@/types/usageDashboard";
import { ProviderApiKeyRow } from "./ProviderApiKeyRow";
import {
  credentialErrorCodeOf,
  credentialErrorCopy,
} from "./providerCredentialErrors";
import { formatUsd } from "./providerSpendFormat";

interface ProviderApiKeysSectionProps {
  provider: UsageProviderView;
}

export function ProviderApiKeysSection({
  provider,
}: ProviderApiKeysSectionProps) {
  const { t, i18n } = useTranslation();
  const credentials = useSystemProviderCredentialActions();
  const [adding, setAdding] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [deleting, setDeleting] = useState<ProviderApiKeyView | null>(null);
  const [errorCode, setErrorCode] = useState<string | null>(null);

  const locale = i18n.resolvedLanguage ?? i18n.language;
  const keys = provider.apiKeys;
  const total = provider.keyUsageTotal;
  // NVIDIA has no endpoint that can validate a key, so its rows offer no test.
  const canTestConnection = provider.systemPresetKey !== "nvidia-nim-api";

  const run = async (operation: () => Promise<unknown>) => {
    setErrorCode(null);
    try {
      return await operation();
    } catch (cause) {
      setErrorCode(credentialErrorCodeOf(cause));
      return undefined;
    }
  };

  const addKey = async () => {
    const label = newLabel.trim();
    if (!label) return;
    const created = await run(() => credentials.createKey(provider.id, label));
    if (created !== undefined) {
      setNewLabel("");
      setAdding(false);
    }
  };

  const error = credentialErrorCopy(errorCode);

  return (
    <div className="space-y-2">
      {/* The total only earns its space once more than one key contributes to
          it; with a single key it would just repeat that key's own figure. */}
      {provider.supportsKeyUsage && total && keys.length > 1 ? (
        <div className="rounded-lg bg-muted/25 px-3 py-2.5 dark:bg-muted/15">
          <div className="text-xs font-medium text-muted-foreground">
            {t("usageDashboard.providerSpendTitle", {
              name: provider.name,
              defaultValue: `Total across all ${provider.name} keys`,
            })}
          </div>
          <div className="mt-2 flex flex-wrap items-baseline gap-x-6 gap-y-2">
            <div>
              <div className="text-2xl font-semibold leading-none tabular-nums">
                {formatUsd(total.usageTotalUsd, locale) ?? "—"}
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {t("usageDashboard.keySpendTotal", { defaultValue: "Total" })}
              </div>
            </div>
            <div className="flex gap-6">
              <div>
                <div className="text-sm font-medium tabular-nums">
                  {formatUsd(total.usageDailyUsd, locale) ?? "—"}
                </div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {t("usageDashboard.keySpendToday", {
                    defaultValue: "Today (UTC)",
                  })}
                </div>
              </div>
              <div>
                <div className="text-sm font-medium tabular-nums">
                  {formatUsd(total.usageMonthlyUsd, locale) ?? "—"}
                </div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {t("usageDashboard.keySpendMonth", {
                    defaultValue: "This month (UTC)",
                  })}
                </div>
              </div>
            </div>
          </div>
          {/* The total is only as fresh as its stalest input, so it reports the
              oldest fetch rather than the newest. */}
          <div
            className="mt-2 text-xs text-muted-foreground"
            title={new Date(total.fetchedAt * 1000).toLocaleString()}
          >
            {t("usageDashboard.providerSpendFetchedAt", {
              value: relativeTimeAgo(total.fetchedAt * 1000, locale),
              defaultValue: `Oldest figure updated ${relativeTimeAgo(total.fetchedAt * 1000, locale)}`,
            })}
          </div>
          {total.stale ? (
            <div className="mt-2 rounded-md border border-warning/20 bg-warning/10 px-2 py-1 text-xs text-warning">
              {t("usageDashboard.providerSpendStale", {
                defaultValue:
                  "Some keys were replaced; their old figures are excluded from this total.",
              })}
            </div>
          ) : null}
        </div>
      ) : null}

      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-medium text-muted-foreground">
          {t("usageDashboard.apiKeysTitle", { defaultValue: "API keys" })}
        </div>
        {!adding ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 px-2 text-xs"
            disabled={credentials.isPending}
            onClick={() => {
              setErrorCode(null);
              setAdding(true);
            }}
          >
            <Plus className="size-3.5" aria-hidden="true" />
            {t("usageDashboard.addApiKey", { defaultValue: "Add key" })}
          </Button>
        ) : null}
      </div>

      {adding ? (
        <form
          className="flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void addKey();
          }}
        >
          <Input
            autoFocus
            className="h-8"
            aria-label={t("usageDashboard.keyLabel", {
              defaultValue: "Key name",
            })}
            placeholder={t("usageDashboard.keyLabelPlaceholder", {
              defaultValue: "Name this key, e.g. OpenClaw",
            })}
            value={newLabel}
            onChange={(event) => setNewLabel(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setAdding(false);
                setNewLabel("");
              }
            }}
          />
          <Button
            type="submit"
            size="sm"
            className="h-8 shrink-0"
            disabled={credentials.isPending || newLabel.trim().length === 0}
          >
            {t("common.add", { defaultValue: "Add" })}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-8 shrink-0"
            onClick={() => {
              setAdding(false);
              setNewLabel("");
            }}
          >
            {t("common.cancel", { defaultValue: "Cancel" })}
          </Button>
        </form>
      ) : null}

      {keys.length === 0 && !adding ? (
        <div className="rounded-lg border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
          {t("usageDashboard.noApiKeys", {
            defaultValue: "No API key yet. Add one to start tracking spend.",
          })}
        </div>
      ) : null}

      {keys.map((apiKey) => (
        <ProviderApiKeyRow
          key={apiKey.id}
          apiKey={apiKey}
          canTestConnection={canTestConnection}
          supportsKeyUsage={provider.supportsKeyUsage}
          onRequestDelete={setDeleting}
        />
      ))}

      {error ? (
        <div role="alert" className="text-sm text-destructive">
          {t(error.key, { defaultValue: error.fallback })}
        </div>
      ) : null}

      <ConfirmDialog
        isOpen={deleting !== null}
        title={t("confirm.deleteProviderApiKey", {
          defaultValue: "Delete API key",
        })}
        message={t("confirm.deleteProviderApiKeyMessage", {
          name: deleting?.label ?? "",
          defaultValue: `Delete the key "${deleting?.label ?? ""}"? Its recorded spend is removed with it, and anything still using this key stops being tracked.`,
        })}
        confirmText={t("common.delete", { defaultValue: "Delete" })}
        cancelText={t("common.cancel", { defaultValue: "Cancel" })}
        onConfirm={() => {
          const target = deleting;
          setDeleting(null);
          if (target) {
            void run(() =>
              credentials.deleteKey(target.id, target.credentialVersion),
            );
          }
        }}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}
