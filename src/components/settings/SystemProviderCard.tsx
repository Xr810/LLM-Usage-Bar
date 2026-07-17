import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CodexOAuthSection } from "@/components/providers/forms/CodexOAuthSection";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  useSetUsageProviderEnabled,
  useSystemProviderCredentialActions,
} from "@/lib/query/usageDashboard";
import type { UsageProviderView } from "@/types/usageDashboard";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";
import { ProviderDailyBudgetField } from "./ProviderDailyBudgetField";
import { SystemProviderApiKeyDialog } from "./SystemProviderApiKeyDialog";

interface SystemProviderCardProps {
  provider: UsageProviderView;
  targetProviderId?: string;
  onTargetHandled?: () => void;
}

export function SystemProviderCard({
  provider,
  targetProviderId,
  onTargetHandled,
}: SystemProviderCardProps) {
  const { t } = useTranslation();
  const credentials = useSystemProviderCredentialActions();
  const setEnabled = useSetUsageProviderEnabled();
  const [keyDialogOpen, setKeyDialogOpen] = useState(false);
  const [failed, setFailed] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<
    "success" | "failed" | null
  >(null);

  const run = async (operation: () => Promise<unknown>) => {
    setFailed(false);
    try {
      return await operation();
    } catch {
      setFailed(true);
      return undefined;
    }
  };

  const hasUpstreamKey = provider.upstreamCredentialStatus === "configured";

  return (
    <Card data-testid={`system-provider-${provider.id}`}>
      <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
        <div className="space-y-1.5">
          <CardTitle className="text-base">{provider.name}</CardTitle>
          <CardDescription>
            {t("usageDashboard.fixedSystemProvider", {
              defaultValue: "Built-in Provider",
            })}
            {provider.canonicalEndpoint
              ? ` · ${t("usageDashboard.endpointLocked", {
                  defaultValue: "Endpoint locked",
                })}`
              : ""}
          </CardDescription>
        </div>
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
      </CardHeader>
      <CardContent className="space-y-4">
        {provider.systemAuthKind === "codex_oauth" ? (
          <CodexOAuthSection variant="system-card" />
        ) : null}
        {provider.systemAuthKind === "claude_cli" ? (
          <ClaudeCliAuthSection />
        ) : null}
        {provider.systemAuthKind === "provider_api_key" ? (
          <div className="space-y-3 rounded-md border bg-muted/20 p-3">
            {provider.canonicalEndpoint ? (
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t("usageDashboard.endpoint", { defaultValue: "Endpoint" })}
                </div>
                <code className="break-all text-xs">
                  {provider.canonicalEndpoint}
                </code>
              </div>
            ) : null}
            <div className="text-sm">
              {hasUpstreamKey
                ? t("usageDashboard.upstreamKeyConfigured", {
                    defaultValue: "Upstream API key configured",
                  })
                : provider.upstreamCredentialStatus === "unavailable"
                  ? t("usageDashboard.credentialUnavailable", {
                      defaultValue: "Credential unavailable",
                    })
                  : t("usageDashboard.upstreamKeyMissing", {
                      defaultValue: "Upstream API key required",
                    })}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button size="sm" onClick={() => setKeyDialogOpen(true)}>
                {hasUpstreamKey
                  ? t("usageDashboard.replaceApiKey", {
                      defaultValue: "Replace API key",
                    })
                  : t("usageDashboard.setApiKey", {
                      defaultValue: "Set API key",
                    })}
              </Button>
              {hasUpstreamKey ? (
                <>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={credentials.isPending}
                    onClick={() =>
                      void run(async () => {
                        const result = await credentials.testConnection(
                          provider.id,
                          provider.upstreamCredentialVersion,
                        );
                        setConnectionStatus(result.status);
                      })
                    }
                  >
                    {t("usageDashboard.testConnection", {
                      defaultValue: "Test connection",
                    })}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={
                      credentials.isPending ||
                      !provider.canClearUpstreamCredential
                    }
                    onClick={() => {
                      if (
                        window.confirm(
                          t("usageDashboard.clearUpstreamKeyConfirmation", {
                            defaultValue:
                              "Clear this Provider API key? Monitoring that requires it will stop.",
                          }),
                        )
                      ) {
                        void run(() =>
                          credentials.clearApiKey(
                            provider.id,
                            provider.upstreamCredentialVersion,
                          ),
                        );
                      }
                    }}
                  >
                    {t("usageDashboard.clearApiKey", {
                      defaultValue: "Clear API key",
                    })}
                  </Button>
                </>
              ) : null}
            </div>
            {(connectionStatus ?? provider.lastConnectionTestStatus) ? (
              <div className="text-xs text-muted-foreground">
                {(connectionStatus ?? provider.lastConnectionTestStatus) ===
                "success"
                  ? t("usageDashboard.connectionSucceeded", {
                      defaultValue: "Connection succeeded",
                    })
                  : t("usageDashboard.connectionFailed", {
                      defaultValue: "Connection failed",
                    })}
              </div>
            ) : null}
          </div>
        ) : null}

        {provider.billingKind === "metered" ? (
          <ProviderDailyBudgetField
            providerId={provider.id}
            providerName={provider.name}
            value={provider.dailyBudgetUsd}
            targeted={targetProviderId === provider.id}
            onTargetHandled={onTargetHandled}
          />
        ) : null}

        {failed ? (
          <div role="alert" className="text-sm text-destructive">
            {t("usageDashboard.providerActionFailed", {
              defaultValue: "Unable to update this Provider.",
            })}
          </div>
        ) : null}
      </CardContent>

      {provider.systemAuthKind === "provider_api_key" ? (
        <SystemProviderApiKeyDialog
          open={keyDialogOpen}
          provider={provider}
          onOpenChange={setKeyDialogOpen}
          isPending={credentials.isPending}
          onSubmit={(apiKey) =>
            hasUpstreamKey
              ? credentials.replaceApiKey(
                  provider.id,
                  provider.upstreamCredentialVersion,
                  apiKey,
                )
              : credentials.setApiKey(
                  provider.id,
                  provider.upstreamCredentialVersion,
                  apiKey,
                )
          }
        />
      ) : null}
    </Card>
  );
}
