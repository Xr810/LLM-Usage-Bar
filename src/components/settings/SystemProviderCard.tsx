import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
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
import { useSystemProviderCredentialActions } from "@/lib/query/usageDashboard";
import type { UsageProviderView } from "@/types/usageDashboard";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";
import { ProviderDailyBudgetField } from "./ProviderDailyBudgetField";
import { ProviderModelPricingSection } from "./ProviderModelPricingSection";
import { ProviderIcon } from "@/components/ProviderIcon";
import { dashboardProviderIcon } from "@/components/usage-dashboard/usagePresentation";

interface SystemProviderCardProps {
  provider: UsageProviderView;
  showBudget?: boolean;
  targetProviderId?: string;
  onTargetHandled?: () => void;
  /** Asks to take this Provider off the list; the parent confirms first. */
  onRemove: (provider: UsageProviderView) => void;
  isRemovePending?: boolean;
}

export function SystemProviderCard({
  provider,
  showBudget = true,
  targetProviderId,
  onTargetHandled,
  onRemove,
  isRemovePending = false,
}: SystemProviderCardProps) {
  const { t } = useTranslation();
  const credentials = useSystemProviderCredentialActions();
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
        {/* A card is on this list because the user put it there, so the only
            action its header needs is taking it back off. */}
        <Button
          size="icon"
          variant="ghost"
          className="size-8 shrink-0 hover:bg-destructive/10 hover:text-destructive"
          disabled={isRemovePending}
          aria-label={t("usageDashboard.removeProvider", {
            name: provider.name,
            defaultValue: `Remove ${provider.name}`,
          })}
          onClick={() => onRemove(provider)}
        >
          <Trash2 className="size-4" aria-hidden="true" />
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

        {provider.billingKind === "metered" ? (
          <ProviderModelPricingSection
            providerId={provider.id}
            credentialVersion={provider.upstreamCredentialVersion}
            providerName={provider.name}
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
