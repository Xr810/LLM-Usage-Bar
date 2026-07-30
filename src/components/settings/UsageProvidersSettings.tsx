import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { UsageProviderDialog } from "@/components/usage-dashboard/UsageProviderDialog";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { SystemProviderCard } from "./SystemProviderCard";
import {
  useDeleteUsageProvider,
  useSaveUsageProvider,
  useSetUsageProviderEnabled,
  useUsageProviders,
} from "@/lib/query/usageDashboard";
import type { UsageProviderView } from "@/types/usageDashboard";
import { ProviderDailyBudgetField } from "./ProviderDailyBudgetField";
import { ProviderIcon } from "@/components/ProviderIcon";
import { dashboardProviderIcon } from "@/components/usage-dashboard/usagePresentation";

const SYSTEM_PROVIDER_ORDER = [
  "chatgpt-subscription",
  "claude-subscription",
  "openai-api",
  "anthropic-api",
  "openrouter-api",
  "gemini-api",
  "xai-api",
  "deepseek-api",
  "kimi-api",
  "glm-api",
  "qwen-api",
  "minimax-api",
  "mistral-api",
  "groq-api",
  "together-api",
  "fireworks-api",
  "perplexity-api",
  "siliconflow-api",
  "nvidia-nim-api",
  "cerebras-api",
];

function providerMatchesSearch(
  provider: UsageProviderView,
  normalizedQuery: string,
): boolean {
  if (!normalizedQuery) return true;
  return [
    provider.name,
    provider.systemPresetKey,
    provider.canonicalEndpoint,
    provider.productGroupId,
  ]
    .filter((value): value is string => Boolean(value))
    .join(" ")
    .toLocaleLowerCase()
    .includes(normalizedQuery);
}

interface UsageProvidersSettingsProps {
  targetProviderId?: string;
  onTargetHandled?: () => void;
}

export function UsageProvidersSettings({
  targetProviderId,
  onTargetHandled,
}: UsageProvidersSettingsProps = {}) {
  const { t } = useTranslation();
  const providersQuery = useUsageProviders();
  const saveProvider = useSaveUsageProvider();
  const deleteProvider = useDeleteUsageProvider();
  const setEnabled = useSetUsageProviderEnabled();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<UsageProviderView | null>(null);
  const [deleting, setDeleting] = useState<UsageProviderView | null>(null);
  const [providerSearch, setProviderSearch] = useState("");
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
  const normalizedProviderSearch = providerSearch.trim().toLocaleLowerCase();
  const visibleSystemProviders = useMemo(
    () =>
      systemProviders.filter((provider) =>
        providerMatchesSearch(provider, normalizedProviderSearch),
      ),
    [normalizedProviderSearch, systemProviders],
  );
  const visibleCustomProviders = useMemo(
    () =>
      customProviders.filter((provider) =>
        providerMatchesSearch(provider, normalizedProviderSearch),
      ),
    [customProviders, normalizedProviderSearch],
  );
  const hasVisibleProviders =
    visibleSystemProviders.length + visibleCustomProviders.length > 0;

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader className="space-y-4">
          <div className="space-y-1.5">
            <CardTitle className="text-base">
              {t("usageDashboard.providers", { defaultValue: "Providers" })}
            </CardTitle>
            <CardDescription>
              {t("usageDashboard.providersSettingsDescription", {
                defaultValue:
                  "Manage Provider accounts, monitoring sources, refresh settings, and budgets.",
              })}
            </CardDescription>
          </div>
          <div className="space-y-2">
            <div className="relative">
              <Search
                aria-hidden="true"
                className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                type="search"
                value={providerSearch}
                onChange={(event) => setProviderSearch(event.target.value)}
                aria-label={t("usageDashboard.searchProviders", {
                  defaultValue: "Search Providers",
                })}
                placeholder={t("usageDashboard.searchProvidersPlaceholder", {
                  defaultValue: "Search by Provider name or endpoint...",
                })}
                className="pl-9"
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {t("usageDashboard.providerCatalogHint", {
                defaultValue:
                  "Search the built-in catalog, then sign in or save an API key. Usage, balance, and quota still depend on the monitoring sources each Provider exposes.",
              })}
            </p>
          </div>
        </CardHeader>
      </Card>

      {providersQuery.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : null}

      {visibleSystemProviders.map((provider) => (
        <SystemProviderCard
          key={provider.id}
          provider={provider}
          targetProviderId={targetProviderId}
          onTargetHandled={onTargetHandled}
        />
      ))}

      {!providersQuery.isLoading &&
      normalizedProviderSearch &&
      !hasVisibleProviders ? (
        <Card className="border-dashed">
          <CardContent className="p-6 text-center text-sm text-muted-foreground">
            {t("usageDashboard.noMatchingProviders", {
              defaultValue: "No Providers match this search.",
            })}
          </CardContent>
        </Card>
      ) : null}

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
                "Add or edit Provider accounts and their monitoring metadata.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          {visibleCustomProviders.map((provider) => {
            const { icon, iconColor } = dashboardProviderIcon(provider);
            return (
              <div
                key={provider.id}
                data-testid={`provider-config-${provider.id}`}
                className="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-muted/25 px-3 py-3 dark:bg-muted/15"
              >
                <div className="flex min-w-0 items-center gap-3">
                  <ProviderIcon
                    icon={icon}
                    color={iconColor}
                    name={provider.name}
                    size={28}
                    className="shrink-0 rounded-lg border border-border/50"
                  />
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">
                      {provider.name}
                    </div>
                    <div className="truncate text-xs text-muted-foreground">
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
                </div>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 px-2.5 text-xs"
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
                    variant="ghost"
                    className="h-8 px-2.5 text-xs"
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
                  <Button
                    size="sm"
                    variant="destructive"
                    className="h-8 px-2.5 text-xs"
                    disabled={deleteProvider.isPending}
                    aria-label={`${t("common.delete", { defaultValue: "Delete" })} ${provider.name}`}
                    onClick={() => setDeleting(provider)}
                  >
                    {t("common.delete", { defaultValue: "Delete" })}
                  </Button>
                </div>
                {provider.billingKind === "metered" ? (
                  <div className="w-full">
                    <ProviderDailyBudgetField
                      providerId={provider.id}
                      providerName={provider.name}
                      value={provider.dailyBudgetUsd}
                      targeted={targetProviderId === provider.id}
                      onTargetHandled={onTargetHandled}
                    />
                  </div>
                ) : null}
              </div>
            );
          })}
          {!providersQuery.isLoading &&
          !normalizedProviderSearch &&
          customProviders.length === 0 ? (
            <div className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
              {t("usageDashboard.noCustomProvidersConfigured", {
                defaultValue: "No custom Providers configured",
              })}
            </div>
          ) : null}
          <Button
            variant="outline"
            className="h-auto w-full border-dashed py-4"
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
          >
            {t("usageDashboard.addProvider", { defaultValue: "Add Provider" })}
          </Button>
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
      <ConfirmDialog
        isOpen={deleting !== null}
        title={t("confirm.deleteProvider", {
          defaultValue: "Delete Provider",
        })}
        message={t("confirm.deleteProviderMessage", {
          name: deleting?.name ?? "",
          defaultValue: `Are you sure you want to delete provider "${deleting?.name ?? ""}"? This action cannot be undone.`,
        })}
        confirmText={t("common.delete", { defaultValue: "Delete" })}
        onConfirm={() => {
          const provider = deleting;
          setDeleting(null);
          if (provider) {
            void run(() => deleteProvider.mutateAsync(provider.id));
          }
        }}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}
