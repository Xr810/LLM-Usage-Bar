import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { UsageProviderDialog } from "@/components/usage-dashboard/UsageProviderDialog";
import { SystemProviderCard } from "./SystemProviderCard";
import {
  useSaveUsageProvider,
  useSetUsageProviderEnabled,
  useUsageProviders,
} from "@/lib/query/usageDashboard";
import type { UsageProviderView } from "@/types/usageDashboard";

const SYSTEM_PROVIDER_ORDER = [
  "chatgpt-subscription",
  "claude-subscription",
  "openai-api",
  "anthropic-api",
  "openrouter-api",
];

export function UsageProvidersSettings() {
  const { t } = useTranslation();
  const providersQuery = useUsageProviders();
  const saveProvider = useSaveUsageProvider();
  const setEnabled = useSetUsageProviderEnabled();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<UsageProviderView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const queryErrors = providersQuery.error
    ? [
        providersQuery.error instanceof Error
          ? providersQuery.error.message
          : String(providersQuery.error),
      ]
    : [];
  const providers = providersQuery.data ?? [];
  const systemProviders = providers
    .filter(
      (provider) =>
        provider.systemPresetKey !== null &&
        provider.systemPresetKey !== undefined,
    )
    .sort(
      (left, right) =>
        SYSTEM_PROVIDER_ORDER.indexOf(left.systemPresetKey ?? "") -
        SYSTEM_PROVIDER_ORDER.indexOf(right.systemPresetKey ?? ""),
    );
  const customProviders = providers.filter(
    (provider) =>
      provider.systemPresetKey === null ||
      provider.systemPresetKey === undefined,
  );

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
          <div className="space-y-1.5">
            <CardTitle className="text-base">
              {t("usageDashboard.providers", { defaultValue: "Providers" })}
            </CardTitle>
            <CardDescription>
              {t("usageDashboard.providersSettingsDescription", {
                defaultValue:
                  "Manage Provider billing identity and data-source metadata. Agent membership is managed only from Agents.",
              })}
            </CardDescription>
          </div>
          <Button
            size="sm"
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
          >
            {t("usageDashboard.addProvider", { defaultValue: "Add Provider" })}
          </Button>
        </CardHeader>
      </Card>

      {providersQuery.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : null}

      {systemProviders.map((provider) => (
        <SystemProviderCard key={provider.id} provider={provider} />
      ))}

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("usageDashboard.customProviders", {
              defaultValue: "Custom Providers",
            })}
          </CardTitle>
          <CardDescription>
            {t("usageDashboard.customProvidersDescription", {
              defaultValue:
                "Add or edit Providers with your own routing and metadata.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          {customProviders.map((provider) => (
            <div
              key={provider.id}
              data-testid={`provider-config-${provider.id}`}
              className="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3"
            >
              <div>
                <div className="font-medium">{provider.name}</div>
                <div className="text-xs text-muted-foreground">
                  {provider.billingKind === "subscription"
                    ? t("usageDashboard.subscription", {
                        defaultValue: "Subscription",
                      })
                    : t("usageDashboard.metered", {
                        defaultValue: "Metered",
                      })}{" "}
                  · {provider.productGroupId} ·{" "}
                  {provider.enabled
                    ? t("common.enabled", { defaultValue: "Enabled" })
                    : t("common.disabled", { defaultValue: "Disabled" })}
                </div>
              </div>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  aria-label={t("usageDashboard.editProvider", {
                    name: provider.name,
                    defaultValue: `Edit ${provider.name}`,
                  })}
                  onClick={() => {
                    setEditing(provider);
                    setDialogOpen(true);
                  }}
                >
                  {t("common.edit", { defaultValue: "Edit" })}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={setEnabled.isPending}
                  aria-label={t(
                    provider.enabled
                      ? "usageDashboard.disableProvider"
                      : "usageDashboard.enableProvider",
                    {
                      name: provider.name,
                      defaultValue: `${provider.enabled ? "Disable" : "Enable"} ${provider.name}`,
                    },
                  )}
                  onClick={() =>
                    void run(() =>
                      setEnabled.mutateAsync({
                        providerId: provider.id,
                        enabled: !provider.enabled,
                      }),
                    )
                  }
                >
                  {provider.enabled
                    ? t("common.disable", { defaultValue: "Disable" })
                    : t("common.enable", { defaultValue: "Enable" })}
                </Button>
              </div>
            </div>
          ))}
          {!providersQuery.isLoading && customProviders.length === 0 ? (
            <div className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
              {t("usageDashboard.noCustomProvidersConfigured", {
                defaultValue: "No custom Providers configured",
              })}
            </div>
          ) : null}
        </CardContent>
      </Card>

      {[...queryErrors, ...(error ? [error] : [])].map((message) => (
        <div key={message} role="alert" className="text-sm text-destructive">
          {message}
        </div>
      ))}

      <UsageProviderDialog
        open={dialogOpen}
        onOpenChange={(nextOpen) => {
          setDialogOpen(nextOpen);
          if (!nextOpen) setEditing(null);
        }}
        provider={editing}
        onSave={(input) => saveProvider.mutateAsync(input)}
        isPending={saveProvider.isPending}
      />
    </div>
  );
}
