import { useMemo, useState } from "react";
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
import {
  useDashboardModules,
  useSaveDashboardModule,
  useSaveUsageProvider,
  useSetUsageProviderEnabled,
  useUsageProviders,
} from "@/lib/query/usageDashboard";
import type {
  DashboardModuleView,
  UsageProviderView,
} from "@/types/usageDashboard";

export function UsageProvidersSettings() {
  const { t } = useTranslation();
  const providersQuery = useUsageProviders();
  const modulesQuery = useDashboardModules();
  const saveProvider = useSaveUsageProvider();
  const setEnabled = useSetUsageProviderEnabled();
  const saveModule = useSaveDashboardModule();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<UsageProviderView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const modules = useMemo(
    () =>
      [...(modulesQuery.data ?? [])].sort((a, b) => a.sortOrder - b.sortOrder),
    [modulesQuery.data],
  );
  const subscriptionModules = modules.filter(
    (module) => module.kind === "subscription",
  );

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const createModule = async (name: string): Promise<DashboardModuleView> =>
    saveModule.mutateAsync({
      id: null,
      name,
      kind: "subscription",
      sortOrder:
        modules.reduce(
          (maximum, module) => Math.max(maximum, module.sortOrder),
          0,
        ) + 1,
      visible: true,
    });

  const queryErrors = [providersQuery.error, modulesQuery.error]
    .filter((cause) => cause != null)
    .map((cause) => (cause instanceof Error ? cause.message : String(cause)));

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
                  "Manage billing identity, data sources, subscription membership, and credentials.",
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
        <CardContent className="space-y-2">
          {providersQuery.isLoading ? (
            <div>{t("common.loading", { defaultValue: "Loading" })}</div>
          ) : null}
          {(providersQuery.data ?? []).map((provider) => (
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
                  aria-label={t("usageDashboard.toggleProvider", {
                    action: provider.enabled ? "Disable" : "Enable",
                    name: provider.name,
                    defaultValue: `${provider.enabled ? "Disable" : "Enable"} ${provider.name}`,
                  })}
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
          {!providersQuery.isLoading &&
          (providersQuery.data ?? []).length === 0 ? (
            <div className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
              {t("usageDashboard.noProvidersConfigured", {
                defaultValue: "No Providers configured",
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
        dashboardModules={subscriptionModules}
        onCreateModule={createModule}
        onSave={(input) => saveProvider.mutateAsync(input)}
        isPending={saveProvider.isPending || saveModule.isPending}
      />
    </div>
  );
}
