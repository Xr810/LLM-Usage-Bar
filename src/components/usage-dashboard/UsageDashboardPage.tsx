import { useMemo, useState } from "react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { UsageDateRangePicker } from "@/components/usage/UsageDateRangePicker";
import { resolveUsageRange } from "@/lib/usageRange";
import type { UsageRangeSelection } from "@/types/usage";
import type { UsageProviderView } from "@/types/usageDashboard";
import {
  useRefreshProviderQuota,
  useRouteBindings,
  useSaveUsageProvider,
  useSetRouteBinding,
  useSetUsageProviderEnabled,
  useSyncProviderSessionUsage,
  useUsageDashboard,
  useUsageProviders,
} from "@/lib/query/usageDashboard";
import {
  useIsProxyRunning,
  useStartProxyServer,
  useStopProxyServer,
} from "@/lib/query/proxy";
import { ProductUsageGroup } from "./ProductUsageGroup";
import { RouteBindingsPanel } from "./RouteBindingsPanel";
import { UsageProviderDialog } from "./UsageProviderDialog";
import { useTranslation } from "react-i18next";

export function UsageDashboardPage() {
  const { t } = useTranslation();
  const [selection, setSelection] = useState<UsageRangeSelection>({
    preset: "today",
  });
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<UsageProviderView | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const range = useMemo(() => resolveUsageRange(selection), [selection]);
  const dashboard = useUsageDashboard(range.startDate, range.endDate);
  const providers = useUsageProviders();
  const bindings = useRouteBindings();
  const saveProvider = useSaveUsageProvider();
  const setEnabled = useSetUsageProviderEnabled();
  const setBinding = useSetRouteBinding();
  const refreshQuota = useRefreshProviderQuota();
  const syncSession = useSyncProviderSessionUsage();
  const proxyRunning = useIsProxyRunning();
  const startProxy = useStartProxyServer();
  const stopProxy = useStopProxyServer();

  const sync = async (providerId: string) => {
    const result = await syncSession.mutateAsync(providerId);
    setWarnings(result.warnings);
  };

  return (
    <div className="space-y-4 pb-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-xl font-semibold">
            {t("usageDashboard.title", { defaultValue: "Usage dashboard" })}
          </h2>
          <p className="text-sm text-muted-foreground">
            {t("usageDashboard.description", {
              defaultValue:
                "Provider-aware tokens, costs and subscription quota",
            })}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {(["today", "7d", "30d"] as const).map((preset) => (
            <Button
              key={preset}
              size="sm"
              variant={selection.preset === preset ? "default" : "outline"}
              aria-pressed={selection.preset === preset}
              onClick={() => setSelection({ preset })}
            >
              {preset === "today"
                ? "Today"
                : preset === "7d"
                  ? "7 days"
                  : "30 days"}
            </Button>
          ))}
          <UsageDateRangePicker
            selection={selection}
            onApply={setSelection}
            triggerLabel="Custom range"
          />
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
          >
            {t("usageDashboard.addProvider", { defaultValue: "Add Provider" })}
          </Button>
          <Button
            size="sm"
            onClick={() =>
              void (proxyRunning.data
                ? stopProxy.mutateAsync()
                : startProxy.mutateAsync())
            }
          >
            {proxyRunning.data
              ? t("usageDashboard.stopProxy", { defaultValue: "Stop proxy" })
              : t("usageDashboard.startProxy", { defaultValue: "Start proxy" })}
          </Button>
        </div>
      </div>

      {[...(dashboard.data?.warnings ?? []), ...warnings].map((warning) => (
        <Alert key={warning}>
          <AlertDescription>{warning}</AlertDescription>
        </Alert>
      ))}

      <RouteBindingsPanel
        providers={providers.data ?? []}
        bindings={bindings.data ?? []}
        onSave={(protocol, providerId) =>
          setBinding.mutateAsync({ protocol, providerId })
        }
      />

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("usageDashboard.providers", { defaultValue: "Providers" })}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {(providers.data ?? []).map((provider) => (
            <div
              key={provider.id}
              data-testid={`provider-config-${provider.id}`}
              className="flex items-center justify-between gap-3 rounded-lg border p-3"
            >
              <div>
                <div className="font-medium">{provider.name}</div>
                <div className="text-xs text-muted-foreground">
                  {provider.billingKind} · {provider.productGroupId}
                </div>
              </div>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
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
                  onClick={() =>
                    void setEnabled.mutateAsync({
                      providerId: provider.id,
                      enabled: !provider.enabled,
                    })
                  }
                >
                  {provider.enabled
                    ? t("common.disable", { defaultValue: "Disable" })
                    : t("common.enable", { defaultValue: "Enable" })}
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      {dashboard.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : null}
      {dashboard.data?.productGroups.map((product) => (
        <ProductUsageGroup
          key={product.productGroupId}
          product={product}
          startAt={range.startDate}
          endAt={range.endDate}
          onRefreshQuota={(id) => refreshQuota.mutateAsync(id)}
          onSyncSessions={sync}
        />
      ))}
      {!dashboard.isLoading && dashboard.data?.productGroups.length === 0 ? (
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          {t("usageDashboard.empty", {
            defaultValue: "No usage providers yet",
          })}
        </div>
      ) : null}

      <UsageProviderDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        provider={editing}
        onSave={(input) => saveProvider.mutateAsync(input)}
      />
    </div>
  );
}
