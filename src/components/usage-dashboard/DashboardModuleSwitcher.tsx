import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { MoreHorizontal } from "lucide-react";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
    <div className="flex min-w-0 items-center justify-center gap-2">
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
      {visibleAgents.length > 4 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="outline"
              aria-label={t("dashboardAgents.more", {
                defaultValue: "More Agents",
              })}
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {visibleAgents.map((agent) => (
              <DropdownMenuItem
                key={agent.id}
                aria-current={agent.id === selectedAgentId ? "page" : undefined}
                onSelect={() => onSelect(agent.id)}
              >
                {agent.name}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}
