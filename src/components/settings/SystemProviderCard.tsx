import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { CodexOAuthSection } from "@/components/providers/forms/CodexOAuthSection";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { UsageProviderView } from "@/types/usageDashboard";
import { ClaudeCliAuthSection } from "./ClaudeCliAuthSection";
import { ProviderApiKeysSection } from "./ProviderApiKeysSection";
import { firstConfiguredKey } from "./providerKeys";
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
  const { icon, iconColor } = dashboardProviderIcon(provider);
  const catalogueKey = firstConfiguredKey(provider);

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
          <ProviderApiKeysSection provider={provider} />
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
            credentialKeyId={catalogueKey?.id ?? null}
            credentialVersion={catalogueKey?.credentialVersion ?? 0}
            providerName={provider.name}
          />
        ) : null}
      </CardContent>
    </Card>
  );
}
