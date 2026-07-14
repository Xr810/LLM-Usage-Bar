import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  useIsProxyRunning,
  useStartProxyServer,
  useStopProxyServer,
} from "@/lib/query/proxy";
import { useAgentModules } from "@/lib/query/usageDashboard";
import { AgentProxySetupPanel } from "./AgentProxySetupPanel";

export function ProxyRoutingSettings() {
  const { t } = useTranslation();
  const proxyRunning = useIsProxyRunning();
  const startProxy = useStartProxyServer();
  const stopProxy = useStopProxyServer();
  const agentsQuery = useAgentModules();
  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [error, setError] = useState<string | null>(null);

  const agents = useMemo(
    () =>
      [...(agentsQuery.data ?? [])]
        .filter((agent) => agent.archivedAt == null)
        .sort((a, b) => a.sortOrder - b.sortOrder),
    [agentsQuery.data],
  );

  useEffect(() => {
    if (!agents.some((agent) => agent.id === selectedAgentId)) {
      setSelectedAgentId(agents[0]?.id ?? "");
    }
  }, [agents, selectedAgentId]);

  const selectedAgent = agents.find((agent) => agent.id === selectedAgentId);
  const proxyPending = startProxy.isPending || stopProxy.isPending;

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const queryErrors = [proxyRunning.error, agentsQuery.error]
    .filter((cause) => cause != null)
    .map((cause) => (cause instanceof Error ? cause.message : String(cause)));

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

      <Label className="block space-y-1">
        <span>{t("agentProxySetup.agent", { defaultValue: "Agent" })}</span>
        <select
          aria-label={t("agentProxySetup.agent", { defaultValue: "Agent" })}
          className="h-9 w-full rounded-md border border-input bg-background px-3"
          value={selectedAgent?.id ?? ""}
          disabled={agentsQuery.isLoading || agents.length === 0}
          onChange={(event) => setSelectedAgentId(event.target.value)}
        >
          {agents.map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.name}
            </option>
          ))}
        </select>
      </Label>

      {selectedAgent ? <AgentProxySetupPanel agent={selectedAgent} /> : null}
    </div>
  );
}
