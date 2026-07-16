import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { AgentModuleView } from "@/types/usageDashboard";

interface AgentSwitcherProps {
  agents: AgentModuleView[];
  selectedAgentId: string;
  onSelect: (agentModuleId: string) => void;
}

export function AgentSwitcher({
  agents,
  selectedAgentId,
  onSelect,
}: AgentSwitcherProps) {
  const { t } = useTranslation();
  const visibleAgents = useMemo(
    () =>
      [...agents]
        .filter((agent) => agent.visible && agent.archivedAt == null)
        .sort((left, right) => left.sortOrder - right.sortOrder),
    [agents],
  );

  if (!visibleAgents.length) return null;

  return (
    <div className="flex min-w-0 items-center justify-center">
      <div className="min-w-0 overflow-x-auto">
        <Tabs value={selectedAgentId} onValueChange={onSelect}>
          <TabsList
            aria-label={t("dashboardAgents.navigation", {
              defaultValue: "Agents",
            })}
            className="w-max max-w-none justify-start"
          >
            {visibleAgents.map((agent) => (
              <TabsTrigger
                key={agent.id}
                value={agent.id}
                className="min-w-0 whitespace-nowrap"
              >
                {agent.name}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
    </div>
  );
}
