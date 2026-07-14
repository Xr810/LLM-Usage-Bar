import { useTranslation } from "react-i18next";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useAgentProxySetupInfo } from "@/lib/query/usageDashboard";
import type {
  AgentModuleView,
  BindingCredentialStatus,
} from "@/types/usageDashboard";

interface AgentProxySetupPanelProps {
  agent: AgentModuleView;
}

function publicCredentialStatus(
  status: BindingCredentialStatus,
  t: ReturnType<typeof useTranslation>["t"],
) {
  if (status === "configured") {
    return t("dashboardAgents.credentialConfigured", {
      defaultValue: "Configured",
    });
  }
  if (status === "missing") {
    return t("dashboardAgents.credentialMissing", { defaultValue: "Missing" });
  }
  if (status === "not_required") {
    return t("dashboardAgents.noApiKeyRequired", {
      defaultValue: "No API key required",
    });
  }
  return t("dashboardAgents.credentialUnavailable", {
    defaultValue: "Unavailable",
  });
}

export function AgentProxySetupPanel({ agent }: AgentProxySetupPanelProps) {
  const { t } = useTranslation();
  const setup = useAgentProxySetupInfo(agent.id);

  if (setup.isLoading) {
    return <div>{t("common.loading", { defaultValue: "Loading" })}</div>;
  }
  if (setup.error) {
    return (
      <div role="alert" className="text-sm text-destructive">
        {setup.error instanceof Error
          ? setup.error.message
          : String(setup.error)}
      </div>
    );
  }
  if (!setup.data || setup.data.agentModuleId !== agent.id) return null;

  const hasClaudeRoute = setup.data.routes.some(
    (route) =>
      route.protocol === "claude" || route.protocol === "claude-desktop",
  );

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("agentProxySetup.namedTitle", {
            name: agent.name,
            defaultValue: `${agent.name} setup`,
          })}
        </CardTitle>
        <CardDescription>
          {t("agentProxySetup.readOnly", {
            defaultValue: "The app does not edit Agent configuration.",
          })}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-1 text-sm">
          <span className="font-medium">
            {t("agentProxySetup.proxyOrigin", { defaultValue: "Proxy origin" })}
          </span>
          <code className="break-all rounded bg-muted px-2 py-1">
            {setup.data.proxyOrigin}
          </code>
        </div>

        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {t("agentProxySetup.routes", { defaultValue: "Local routes" })}
          </h3>
          {setup.data.routes.map((route) => (
            <div
              key={`${route.providerId}-${route.protocol ?? "none"}-${route.localBaseUrl ?? "none"}`}
              className="grid gap-2 rounded-lg border p-3 text-sm"
            >
              <div className="font-medium">{route.providerId}</div>
              <dl className="grid gap-1 text-muted-foreground sm:grid-cols-[150px_1fr]">
                <dt>
                  {t("agentProxySetup.protocol", { defaultValue: "Protocol" })}
                </dt>
                <dd>
                  {route.protocol ??
                    t("common.unavailable", { defaultValue: "Unavailable" })}
                </dd>
                <dt>
                  {t("agentProxySetup.localBaseUrl", {
                    defaultValue: "Local base URL",
                  })}
                </dt>
                <dd className="break-all">
                  {route.localBaseUrl ??
                    t("common.unavailable", { defaultValue: "Unavailable" })}
                </dd>
                <dt>
                  {t("agentProxySetup.credentialPlacement", {
                    defaultValue: "Credential placement",
                  })}
                </dt>
                <dd>
                  {route.credentialPlacements.length > 0
                    ? route.credentialPlacements.join(", ")
                    : t("common.unavailable", { defaultValue: "Unavailable" })}
                </dd>
                <dt>
                  {t("agentProxySetup.credentialStatus", {
                    defaultValue: "Credential status",
                  })}
                </dt>
                <dd>{publicCredentialStatus(route.credentialStatus, t)}</dd>
              </dl>
            </div>
          ))}
          {setup.data.routes.length === 0 ? (
            <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
              {t("agentProxySetup.noRoutes", {
                defaultValue: "No local routes",
              })}
            </div>
          ) : null}
        </div>

        {hasClaudeRoute ? (
          <div className="rounded-lg border bg-muted/40 p-3 text-sm">
            {t("agentProxySetup.claudeDesktopCredential", {
              defaultValue:
                "Claude Desktop: use x-api-key for the binding key. Authorization is the gateway token; it is not an equivalent key location.",
            })}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
