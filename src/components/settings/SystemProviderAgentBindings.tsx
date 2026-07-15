import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { copyText } from "@/lib/clipboard";
import {
  useAgentProviderLocalKeyActions,
  useDeleteAgentProviderBinding,
  useSaveAgentProviderBinding,
} from "@/lib/query/usageDashboard";
import type {
  AgentProviderBindingView,
  UsageProviderView,
} from "@/types/usageDashboard";

const AGENT_NAMES: Record<string, string> = {
  codex: "Codex",
  "claude-code": "Claude Code",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  hermes: "Hermes",
};

interface SystemProviderAgentBindingsProps {
  provider: UsageProviderView;
}

export function SystemProviderAgentBindings({
  provider,
}: SystemProviderAgentBindingsProps) {
  const { t } = useTranslation();
  const saveBinding = useSaveAgentProviderBinding();
  const deleteBinding = useDeleteAgentProviderBinding();
  const localKeys = useAgentProviderLocalKeyActions();
  const [failed, setFailed] = useState(false);
  const [copiedAgent, setCopiedAgent] = useState<string | null>(null);

  const run = async (operation: () => Promise<unknown>) => {
    setFailed(false);
    try {
      await operation();
    } catch {
      setFailed(true);
    }
  };

  const changeBinding = (
    agentModuleId: string,
    binding: AgentProviderBindingView | undefined,
    checked: boolean,
  ) => {
    if (checked && !binding) {
      return run(() =>
        saveBinding.mutateAsync({
          id: null,
          agentModuleId,
          providerId: provider.id,
          enabled: true,
        }),
      );
    }
    if (!checked && binding) {
      return run(() =>
        deleteBinding.mutateAsync({
          bindingId: binding.id,
          expectedVersion: binding.credentialVersion,
        }),
      );
    }
    return Promise.resolve();
  };

  const copyLocalKey = async (
    binding: AgentProviderBindingView,
    rotate: boolean,
  ) => {
    setFailed(false);
    let transientKey = "";
    try {
      const reveal = rotate
        ? await localKeys.rotate(binding.id, binding.credentialVersion)
        : await localKeys.reveal(binding.id, binding.credentialVersion);
      transientKey = reveal.localKey;
      try {
        await copyText(transientKey);
        setCopiedAgent(binding.agentModuleId);
      } finally {
        reveal.localKey = "";
        transientKey = "";
      }
    } catch {
      transientKey = "";
      setFailed(true);
    }
  };

  const pending =
    saveBinding.isPending || deleteBinding.isPending || localKeys.isPending;

  return (
    <div className="space-y-2">
      <div className="text-sm font-medium">
        {t("usageDashboard.agentBindings", {
          defaultValue: "Agent bindings",
        })}
      </div>
      {provider.compatibleAgentModuleIds.map((agentModuleId) => {
        const binding = provider.bindings.find(
          (candidate) => candidate.agentModuleId === agentModuleId,
        );
        const name = AGENT_NAMES[agentModuleId] ?? agentModuleId;
        const checkboxId = `${provider.id}-${agentModuleId}-binding`;
        const stateLabel = !binding
          ? null
          : !provider.enabled
            ? t("usageDashboard.providerDisabled", {
                defaultValue: "Provider disabled",
              })
            : !binding.enabled
              ? t("usageDashboard.requestedDisabled", {
                  defaultValue: "Requested: disabled",
                })
              : binding.effectiveEnabled
                ? t("usageDashboard.effective", { defaultValue: "Effective" })
                : binding.credentialStatus === "missing"
                  ? t("usageDashboard.missingKey", {
                      defaultValue: "Missing key",
                    })
                  : binding.credentialStatus === "unavailable"
                    ? t("usageDashboard.credentialUnavailable", {
                        defaultValue: "Credential unavailable",
                      })
                    : t("usageDashboard.requestedDisconnected", {
                        defaultValue: "Requested · Disconnected",
                      });
        return (
          <div
            key={agentModuleId}
            className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3"
          >
            <div className="flex items-center gap-2">
              <Checkbox
                id={checkboxId}
                checked={Boolean(binding)}
                disabled={pending}
                onCheckedChange={(checked) =>
                  void changeBinding(agentModuleId, binding, checked === true)
                }
              />
              <Label htmlFor={checkboxId}>
                {t("usageDashboard.useAgent", {
                  defaultValue: `Use ${name}`,
                  name,
                })}
              </Label>
              {stateLabel ? (
                <span
                  className="text-xs text-muted-foreground"
                  aria-label={`${name}: ${stateLabel}`}
                >
                  {stateLabel}
                </span>
              ) : null}
            </div>
            {binding && provider.systemAuthKind === "provider_api_key" ? (
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  aria-label={t("usageDashboard.copyAgentLocalKey", {
                    defaultValue: `Copy ${name} local key`,
                    name,
                  })}
                  onClick={() => void copyLocalKey(binding, false)}
                >
                  {copiedAgent === agentModuleId
                    ? t("common.copied", { defaultValue: "Copied" })
                    : t("common.copy", { defaultValue: "Copy key" })}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  aria-label={t("usageDashboard.rotateAgentLocalKey", {
                    defaultValue: `Rotate ${name} local key`,
                    name,
                  })}
                  onClick={() => {
                    if (
                      window.confirm(
                        t("usageDashboard.rotateLocalKeyConfirmation", {
                          defaultValue:
                            "Rotate this local key? The previous key will stop working.",
                        }),
                      )
                    ) {
                      void copyLocalKey(binding, true);
                    }
                  }}
                >
                  {t("usageDashboard.rotateKey", {
                    defaultValue: "Rotate key",
                  })}
                </Button>
              </div>
            ) : null}
          </div>
        );
      })}
      {failed ? (
        <div role="alert" className="text-sm text-destructive">
          {t("usageDashboard.bindingActionFailed", {
            defaultValue: "Unable to update this Agent binding.",
          })}
        </div>
      ) : null}
    </div>
  );
}
