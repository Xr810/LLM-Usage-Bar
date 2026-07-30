import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CodexOAuthSection } from "@/components/providers/forms/CodexOAuthSection";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import { ProviderIcon } from "@/components/ProviderIcon";
import { dashboardProviderIcon } from "@/components/usage-dashboard/usagePresentation";

interface SystemProviderCardProps {
  provider: UsageProviderView;
  showBudget?: boolean;
  targetProviderId?: string;
  onTargetHandled?: () => void;
}

export function SystemProviderCard({
  provider,
  showBudget = true,
  targetProviderId,
  onTargetHandled,
}: SystemProviderCardProps) {
  const { t } = useTranslation();
  const credentials = useSystemProviderCredentialActions();
  const setEnabled = useSetUsageProviderEnabled();
  const [apiKey, setApiKey] = useState("");
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
  const canTestConnection = provider.systemPresetKey !== "nvidia-nim-api";
  const { icon, iconColor } = dashboardProviderIcon(provider);

  const verifyApiKey = async () => {
    const transientKey = apiKey.trim();
    if (!transientKey) return;

    setApiKey("");
    setConnectionStatus(null);
    await run(async () => {
      const updatedProvider = hasUpstreamKey
        ? await credentials.replaceApiKey(
            provider.id,
            provider.upstreamCredentialVersion,
            transientKey,
          )
        : await credentials.setApiKey(
            provider.id,
            provider.upstreamCredentialVersion,
            transientKey,
          );

      if (canTestConnection) {
        const result = await credentials.testConnection(
          provider.id,
          updatedProvider.upstreamCredentialVersion,
        );
        setConnectionStatus(result.status);
      }
    });
  };

  return (
    <Card data-testid={`system-provider-${provider.id}`}>
      <CardHeader className="flex-row items-center justify-between gap-4 space-y-0">
        <div className="flex min-w-0 items-center gap-3">
          <ProviderIcon
            icon={icon}
            color={iconColor}
            name={provider.name}
            size={32}
            className="shrink-0 rounded-lg border border-border/50"
          />
          <div className="min-w-0 space-y-0.5">
            <CardTitle className="text-[15px]">{provider.name}</CardTitle>
            <CardDescription className="text-xs">
              {t("usageDashboard.fixedSystemProvider", {
                defaultValue: "Built-in Provider",
              })}
            </CardDescription>
          </div>
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
          <div className="space-y-2">
            <form
              className="flex items-center gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                void verifyApiKey();
              }}
            >
              <Input
                type="password"
                autoComplete="off"
                aria-label={t("usageDashboard.apiKey", {
                  defaultValue: "API key",
                })}
                placeholder={t("usageDashboard.apiKeyPlaceholder", {
                  defaultValue: "Enter API key",
                })}
                value={apiKey}
                onChange={(event) => {
                  setApiKey(event.target.value);
                  setConnectionStatus(null);
                }}
              />
              <Button
                type="submit"
                size="sm"
                className="shrink-0"
                disabled={credentials.isPending || apiKey.trim().length === 0}
              >
                {t("usageDashboard.verifyApiKey", {
                  defaultValue: "Verify",
                })}
              </Button>
            </form>
            {connectionStatus ? (
              <div className="text-xs text-muted-foreground">
                {connectionStatus === "success"
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

        {provider.billingKind === "metered" && showBudget ? (
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
    </Card>
  );
}
