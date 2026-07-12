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
import { RouteBindingsPanel } from "@/components/usage-dashboard/RouteBindingsPanel";
import {
  useIsProxyRunning,
  useStartProxyServer,
  useStopProxyServer,
} from "@/lib/query/proxy";
import {
  useRouteBindings,
  useSetRouteBinding,
  useUsageProviders,
} from "@/lib/query/usageDashboard";

export function ProxyRoutingSettings() {
  const { t } = useTranslation();
  const proxyRunning = useIsProxyRunning();
  const startProxy = useStartProxyServer();
  const stopProxy = useStopProxyServer();
  const providers = useUsageProviders();
  const bindings = useRouteBindings();
  const setBinding = useSetRouteBinding();
  const [error, setError] = useState<string | null>(null);

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const queryErrors = [proxyRunning.error, providers.error, bindings.error]
    .filter((cause) => cause != null)
    .map((cause) => (cause instanceof Error ? cause.message : String(cause)));
  const proxyPending = startProxy.isPending || stopProxy.isPending;
  const forwardingTitle = t("usageDashboard.proxyForwardingTargets", {
    defaultValue: "Proxy forwarding targets",
  });
  const forwardingDescription = t("usageDashboard.proxyForwardingDescription", {
    defaultValue:
      "These targets decide where requests are forwarded. They are not a dashboard filter.",
  });

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("usageDashboard.localProxy", { defaultValue: "Local proxy" })}
          </CardTitle>
          <CardDescription>
            {t("usageDashboard.localProxyDescription", {
              defaultValue:
                "Start or stop the local forwarding service used by configured clients.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center justify-between gap-3">
          <div className="text-sm">
            {proxyRunning.isLoading
              ? t("common.loading", { defaultValue: "Loading" })
              : proxyRunning.data
                ? t("usageDashboard.proxyRunning", {
                    defaultValue: "Proxy running",
                  })
                : t("usageDashboard.proxyStopped", {
                    defaultValue: "Proxy stopped",
                  })}
          </div>
          <Button
            disabled={
              proxyRunning.isLoading ||
              Boolean(proxyRunning.error) ||
              proxyPending
            }
            onClick={() =>
              void run(() =>
                proxyRunning.data
                  ? stopProxy.mutateAsync()
                  : startProxy.mutateAsync(),
              )
            }
          >
            {proxyRunning.data
              ? t("usageDashboard.stopProxy", { defaultValue: "Stop proxy" })
              : t("usageDashboard.startProxy", {
                  defaultValue: "Start proxy",
                })}
          </Button>
        </CardContent>
      </Card>

      {[...queryErrors, ...(error ? [error] : [])].map((message) => (
        <div key={message} role="alert" className="text-sm text-destructive">
          {message}
        </div>
      ))}

      <RouteBindingsPanel
        title={forwardingTitle}
        description={forwardingDescription}
        providers={providers.data ?? []}
        bindings={bindings.data ?? []}
        onSave={(protocol, providerId) =>
          setBinding.mutateAsync({ protocol, providerId })
        }
        isPending={
          providers.isLoading || bindings.isLoading || setBinding.isPending
        }
      />
    </div>
  );
}
