import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type {
  AgentProviderBindingView,
  UsageProviderView,
} from "@/types/usageDashboard";
import {
  ProtectedBindingKeyDialog,
  type ProtectedBindingKeyMode,
} from "./ProtectedBindingKeyDialog";

interface AgentProviderBindingRowProps {
  binding: AgentProviderBindingView;
  provider?: Pick<UsageProviderView, "id" | "name">;
  isPending?: boolean;
  onToggle: (binding: AgentProviderBindingView) => Promise<unknown>;
  onDelete: (binding: AgentProviderBindingView) => Promise<unknown>;
  onSetApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ) => Promise<unknown>;
  onReplaceApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ) => Promise<unknown>;
  onClearApiKey: (
    bindingId: string,
    expectedVersion: number,
  ) => Promise<unknown>;
}

export function AgentProviderBindingRow({
  binding,
  provider,
  isPending = false,
  onToggle,
  onDelete,
  onSetApiKey,
  onReplaceApiKey,
  onClearApiKey,
}: AgentProviderBindingRowProps) {
  const { t } = useTranslation();
  const [keyMode, setKeyMode] = useState<ProtectedBindingKeyMode | null>(null);
  const providerName = provider?.name ?? binding.providerId;
  const requestedState = binding.enabled
    ? t("common.enabled", { defaultValue: "Enabled" })
    : t("common.disabled", { defaultValue: "Disabled" });
  const effectiveState = binding.effectiveEnabled
    ? t("common.enabled", { defaultValue: "Enabled" })
    : t("common.disabled", { defaultValue: "Disabled" });
  const credentialLabel =
    binding.credentialStatus === "configured"
      ? t("dashboardAgents.credentialConfigured", {
          defaultValue: "Configured",
        })
      : binding.credentialStatus === "missing"
        ? t("dashboardAgents.credentialMissing", { defaultValue: "Missing" })
        : t("dashboardAgents.credentialUnavailable", {
            defaultValue: "Unavailable",
          });

  return (
    <div
      data-testid={`agent-binding-${binding.id}`}
      className="space-y-3 rounded-md border p-3"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="font-medium">{providerName}</div>
          <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
            <span>
              {t("dashboardAgents.requestedState", {
                state: requestedState,
                defaultValue: `Requested: ${requestedState}`,
              })}
            </span>
            <span>
              {t("dashboardAgents.effectiveState", {
                state: effectiveState,
                defaultValue: `Effective: ${effectiveState}`,
              })}
            </span>
          </div>
        </div>
        {binding.credentialStatus === "not_required" ? (
          <span className="text-xs text-muted-foreground">
            {t("dashboardAgents.noApiKeyRequired", {
              defaultValue: "No API key required",
            })}
          </span>
        ) : (
          <Badge variant="outline">{credentialLabel}</Badge>
        )}
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={isPending}
          aria-label={t(
            binding.enabled
              ? "dashboardAgents.disableProvider"
              : "dashboardAgents.enableProvider",
            {
              name: providerName,
              defaultValue: `${binding.enabled ? "Disable" : "Enable"} ${providerName}`,
            },
          )}
          onClick={() => void onToggle(binding)}
        >
          {binding.enabled
            ? t("common.disable", { defaultValue: "Disable" })
            : t("common.enable", { defaultValue: "Enable" })}
        </Button>
        {binding.credentialStatus === "missing" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={isPending}
            onClick={() => setKeyMode("set")}
          >
            {t("dashboardAgents.setApiKey", { defaultValue: "Set API key" })}
          </Button>
        ) : null}
        {binding.credentialStatus === "configured" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={isPending}
            onClick={() => setKeyMode("replace")}
          >
            {t("dashboardAgents.replaceApiKey", {
              defaultValue: "Replace API key",
            })}
          </Button>
        ) : null}
        {binding.credentialStatus === "configured" ||
        (binding.credentialStatus === "unavailable" &&
          binding.canClearCredential) ? (
          <Button
            size="sm"
            variant="outline"
            disabled={isPending}
            onClick={() => setKeyMode("clear")}
          >
            {t("dashboardAgents.clearApiKey", {
              defaultValue: "Clear API key",
            })}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="destructive"
          disabled={isPending}
          aria-label={t("dashboardAgents.deleteBinding", {
            name: providerName,
            defaultValue: `Delete ${providerName} binding`,
          })}
          onClick={() => void onDelete(binding)}
        >
          {t("common.delete", { defaultValue: "Delete" })}
        </Button>
      </div>

      {keyMode ? (
        <ProtectedBindingKeyDialog
          open
          mode={keyMode}
          binding={binding}
          onOpenChange={(open) => {
            if (!open) setKeyMode(null);
          }}
          onSetApiKey={onSetApiKey}
          onReplaceApiKey={onReplaceApiKey}
          onClearApiKey={onClearApiKey}
          isPending={isPending}
        />
      ) : null}
    </div>
  );
}
