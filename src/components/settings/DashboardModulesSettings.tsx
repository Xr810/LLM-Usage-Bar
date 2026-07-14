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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  useAgentModules,
  useAgentProviderBindingCredentialActions,
  useAgentProviderBindings,
  useDeleteAgentModule,
  useDeleteAgentProviderBinding,
  useReorderAgentModules,
  useSaveAgentModule,
  useSaveAgentProviderBinding,
  useSetAgentModuleVisibility,
  useUsageProviders,
} from "@/lib/query/usageDashboard";
import type {
  AgentModuleInput,
  AgentModuleView,
  AgentProviderBindingInput,
  AgentProviderBindingView,
  UsageProviderView,
} from "@/types/usageDashboard";
import { AgentProviderBindingRow } from "./AgentProviderBindingRow";

interface AgentRowProps {
  agent: AgentModuleView;
  index: number;
  count: number;
  providers: UsageProviderView[];
  bindings: AgentProviderBindingView[];
  isPending: boolean;
  onSave: (input: AgentModuleInput) => Promise<unknown>;
  onMove: (index: number, offset: -1 | 1) => Promise<unknown>;
  onVisibility: (agent: AgentModuleView) => Promise<unknown>;
  onDelete: (agentModuleId: string) => Promise<unknown>;
  onSaveBinding: (input: AgentProviderBindingInput) => Promise<unknown>;
  onDeleteBinding: (binding: AgentProviderBindingView) => Promise<unknown>;
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

function AgentRow({
  agent,
  index,
  count,
  providers,
  bindings,
  isPending,
  onSave,
  onMove,
  onVisibility,
  onDelete,
  onSaveBinding,
  onDeleteBinding,
  onSetApiKey,
  onReplaceApiKey,
  onClearApiKey,
}: AgentRowProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(agent.name);
  const [providerId, setProviderId] = useState("");

  useEffect(() => setName(agent.name), [agent.name]);
  const boundProviderIds = new Set(
    bindings.map((binding) => binding.providerId),
  );
  const availableProviders = providers.filter(
    (provider) => !boundProviderIds.has(provider.id),
  );

  return (
    <div
      data-testid={`agent-settings-${agent.id}`}
      className="space-y-4 rounded-lg border p-4"
    >
      <div className="grid gap-3 lg:grid-cols-[minmax(180px,1fr)_auto_auto] lg:items-center">
        <div className="space-y-2">
          {agent.isFixed ? (
            <div className="font-medium">{agent.name}</div>
          ) : (
            <Label>
              <span className="sr-only">
                {t("dashboardAgents.nameFor", {
                  name: agent.name,
                  defaultValue: `Name for ${agent.name}`,
                })}
              </span>
              <Input
                aria-label={t("dashboardAgents.nameFor", {
                  name: agent.name,
                  defaultValue: `Name for ${agent.name}`,
                })}
                value={name}
                disabled={isPending}
                onChange={(event) => setName(event.target.value)}
              />
            </Label>
          )}
          <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
            <span>
              {agent.isFixed
                ? t("dashboardAgents.fixed", { defaultValue: "Fixed Agent" })
                : t("dashboardAgents.custom", {
                    defaultValue: "Custom Agent",
                  })}
            </span>
            <span>
              {t("dashboardAgents.providerCount", {
                count: agent.providerCount,
                defaultValue: `${agent.providerCount} Providers`,
              })}
            </span>
            <span>
              {agent.visible
                ? t("common.visible", { defaultValue: "Visible" })
                : t("common.hidden", { defaultValue: "Hidden" })}
            </span>
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          {!agent.isFixed ? (
            <Button
              size="sm"
              variant="outline"
              disabled={isPending || !name.trim() || name.trim() === agent.name}
              aria-label={t("dashboardAgents.saveNamed", {
                name: agent.name,
                defaultValue: `Save ${agent.name}`,
              })}
              onClick={() =>
                void onSave({
                  id: agent.id,
                  name: name.trim(),
                  sortOrder: agent.sortOrder,
                  visible: agent.visible,
                })
              }
            >
              {t("common.save", { defaultValue: "Save" })}
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="outline"
            disabled={isPending || index === 0}
            aria-label={t("dashboardAgents.moveUp", {
              name: agent.name,
              defaultValue: `Move ${agent.name} up`,
            })}
            onClick={() => void onMove(index, -1)}
          >
            ↑
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={isPending || index === count - 1}
            aria-label={t("dashboardAgents.moveDown", {
              name: agent.name,
              defaultValue: `Move ${agent.name} down`,
            })}
            onClick={() => void onMove(index, 1)}
          >
            ↓
          </Button>
        </div>

        <div className="flex flex-wrap gap-2 lg:justify-end">
          <Button
            size="sm"
            variant="outline"
            disabled={isPending}
            aria-label={t(
              agent.visible
                ? "dashboardAgents.hideNamed"
                : "dashboardAgents.showNamed",
              {
                name: agent.name,
                defaultValue: `${agent.visible ? "Hide" : "Show"} ${agent.name}`,
              },
            )}
            onClick={() => void onVisibility(agent)}
          >
            {agent.visible
              ? t("common.hide", { defaultValue: "Hide" })
              : t("common.show", { defaultValue: "Show" })}
          </Button>
          {!agent.isFixed ? (
            <Button
              size="sm"
              variant="destructive"
              disabled={isPending}
              aria-label={t("dashboardAgents.deleteNamed", {
                name: agent.name,
                defaultValue: `Delete ${agent.name}`,
              })}
              onClick={() => void onDelete(agent.id)}
            >
              {t("common.delete", { defaultValue: "Delete" })}
            </Button>
          ) : null}
        </div>
      </div>

      <div className="space-y-3 border-t pt-4">
        <div className="font-medium">
          {t("dashboardAgents.providerBindings", {
            defaultValue: "Provider bindings",
          })}
        </div>
        {bindings.map((binding) => (
          <AgentProviderBindingRow
            key={binding.id}
            binding={binding}
            provider={providers.find(
              (provider) => provider.id === binding.providerId,
            )}
            isPending={isPending}
            onToggle={(current) =>
              onSaveBinding({
                id: current.id,
                agentModuleId: current.agentModuleId,
                providerId: current.providerId,
                enabled: !current.enabled,
              })
            }
            onDelete={onDeleteBinding}
            onSetApiKey={onSetApiKey}
            onReplaceApiKey={onReplaceApiKey}
            onClearApiKey={onClearApiKey}
          />
        ))}
        <div className="flex flex-wrap items-end gap-2">
          <Label className="min-w-[220px] flex-1">
            <span className="sr-only">
              {t("dashboardAgents.addProviderFor", {
                name: agent.name,
                defaultValue: `Add Provider for ${agent.name}`,
              })}
            </span>
            <select
              aria-label={t("dashboardAgents.addProviderFor", {
                name: agent.name,
                defaultValue: `Add Provider for ${agent.name}`,
              })}
              className="h-9 w-full rounded-md border border-input bg-background px-3"
              value={providerId}
              disabled={isPending}
              onChange={(event) => setProviderId(event.target.value)}
            >
              <option value="">
                {t("dashboardAgents.chooseProvider", {
                  defaultValue: "Choose Provider",
                })}
              </option>
              {availableProviders.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.name}
                </option>
              ))}
            </select>
          </Label>
          <Button
            size="sm"
            variant="outline"
            disabled={isPending || !providerId}
            aria-label={t("dashboardAgents.addBindingFor", {
              name: agent.name,
              defaultValue: `Add binding for ${agent.name}`,
            })}
            onClick={() => {
              const selectedProviderId = providerId;
              setProviderId("");
              void onSaveBinding({
                id: null,
                agentModuleId: agent.id,
                providerId: selectedProviderId,
                enabled: false,
              });
            }}
          >
            {t("dashboardAgents.addBinding", { defaultValue: "Add binding" })}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function AgentsSettings() {
  const { t } = useTranslation();
  const agentsQuery = useAgentModules();
  const providersQuery = useUsageProviders();
  const bindingsQuery = useAgentProviderBindings();
  const saveAgent = useSaveAgentModule();
  const reorderAgents = useReorderAgentModules();
  const setVisibility = useSetAgentModuleVisibility();
  const deleteAgent = useDeleteAgentModule();
  const saveBinding = useSaveAgentProviderBinding();
  const deleteBinding = useDeleteAgentProviderBinding();
  const credentialActions = useAgentProviderBindingCredentialActions();
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const agents = useMemo(
    () =>
      [...(agentsQuery.data ?? [])]
        .filter((agent) => agent.archivedAt == null)
        .sort((left, right) => left.sortOrder - right.sortOrder),
    [agentsQuery.data],
  );
  const providers = providersQuery.data ?? [];
  const bindings = bindingsQuery.data ?? [];
  const isPending =
    saveAgent.isPending ||
    reorderAgents.isPending ||
    setVisibility.isPending ||
    deleteAgent.isPending ||
    saveBinding.isPending ||
    deleteBinding.isPending ||
    credentialActions.isPending;

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      return await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return undefined;
    }
  };

  const move = async (index: number, offset: -1 | 1) => {
    const nextIndex = index + offset;
    if (nextIndex < 0 || nextIndex >= agents.length) return;
    const reordered = [...agents];
    [reordered[index], reordered[nextIndex]] = [
      reordered[nextIndex],
      reordered[index],
    ];
    await run(() =>
      reorderAgents.mutateAsync(reordered.map((agent) => agent.id)),
    );
  };

  const queryErrors = [
    agentsQuery.error,
    providersQuery.error,
    bindingsQuery.error,
  ]
    .filter((cause) => cause != null)
    .map((cause) => (cause instanceof Error ? cause.message : String(cause)));

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("dashboardAgents.settingsTitle", { defaultValue: "Agents" })}
          </CardTitle>
          <CardDescription>
            {t("dashboardAgents.settingsDescription", {
              defaultValue:
                "Manage Agent navigation and explicit Provider bindings. Fixed Agent identities cannot be renamed or deleted.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-end gap-2">
            <Label className="min-w-[220px] flex-1">
              {t("dashboardAgents.customName", {
                defaultValue: "Custom Agent name",
              })}
              <Input
                value={newName}
                disabled={isPending}
                onChange={(event) => setNewName(event.target.value)}
              />
            </Label>
            <Button
              disabled={isPending || !newName.trim()}
              onClick={() => {
                const name = newName.trim();
                setNewName("");
                void run(() =>
                  saveAgent.mutateAsync({
                    id: null,
                    name,
                    sortOrder:
                      agents.reduce(
                        (maximum, agent) => Math.max(maximum, agent.sortOrder),
                        0,
                      ) + 1,
                    visible: true,
                  }),
                );
              }}
            >
              {t("dashboardAgents.createCustom", {
                defaultValue: "Create Custom Agent",
              })}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("dashboardAgents.navigationImpact", {
              defaultValue:
                "Reordering and visibility affect navigation only. They never edit Agent configuration files or move historical usage.",
            })}
          </p>
        </CardContent>
      </Card>

      {[...queryErrors, ...(error ? [error] : [])].map((message) => (
        <div key={message} role="alert" className="text-sm text-destructive">
          {message}
        </div>
      ))}

      {agentsQuery.isLoading ||
      providersQuery.isLoading ||
      bindingsQuery.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : null}
      <div className="space-y-3">
        {agents.map((agent, index) => (
          <AgentRow
            key={agent.id}
            agent={agent}
            index={index}
            count={agents.length}
            providers={providers}
            bindings={bindings.filter(
              (binding) => binding.agentModuleId === agent.id,
            )}
            isPending={isPending}
            onSave={(input) => run(() => saveAgent.mutateAsync(input))}
            onMove={move}
            onVisibility={(current) =>
              run(() =>
                setVisibility.mutateAsync({
                  agentModuleId: current.id,
                  visible: !current.visible,
                }),
              )
            }
            onDelete={(agentModuleId) =>
              run(() => deleteAgent.mutateAsync(agentModuleId))
            }
            onSaveBinding={(input) => run(() => saveBinding.mutateAsync(input))}
            onDeleteBinding={(binding) =>
              run(() =>
                deleteBinding.mutateAsync({
                  bindingId: binding.id,
                  expectedVersion: binding.credentialVersion,
                }),
              )
            }
            onSetApiKey={credentialActions.setApiKey}
            onReplaceApiKey={credentialActions.replaceApiKey}
            onClearApiKey={credentialActions.clearApiKey}
          />
        ))}
      </div>
    </div>
  );
}
