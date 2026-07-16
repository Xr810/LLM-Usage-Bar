import { useEffect, useRef } from "react";
import type { TrayAgentUsageView } from "@/types/trayUsage";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { TabsList, TabsTrigger } from "@/components/ui/tabs";

export function TrayAgentTabs({
  agents,
  selectedAgentId,
  overviewLabel,
  navigationLabel,
  panelId,
  focusRequestKey,
}: {
  agents: TrayAgentUsageView[];
  selectedAgentId: "overview" | string;
  overviewLabel: string;
  navigationLabel: string;
  panelId: string;
  focusRequestKey: number;
}) {
  const selectedTabRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    selectedTabRef.current?.focus();
  }, [focusRequestKey, selectedAgentId]);

  return (
    <div className="shrink-0 border-b border-border">
      <ScrollArea className="w-full whitespace-nowrap">
        <TabsList
          aria-label={navigationLabel}
          className="tray-agent-tab-list h-11 w-max min-w-full justify-start gap-1 rounded-none bg-popover px-2 py-1.5"
        >
          <TabsTrigger
            ref={selectedAgentId === "overview" ? selectedTabRef : undefined}
            value="overview"
            id="tray-agent-tab-overview"
            aria-controls={panelId}
            className="tray-agent-tab h-8 min-w-0 max-w-[148px] px-3 text-xs"
          >
            {overviewLabel}
          </TabsTrigger>
          {agents.map((agent, index) => (
            <TabsTrigger
              key={agent.agentModuleId}
              ref={
                selectedAgentId === agent.agentModuleId
                  ? selectedTabRef
                  : undefined
              }
              value={agent.agentModuleId}
              id={`tray-agent-tab-${index}`}
              aria-controls={panelId}
              title={agent.name}
              className="tray-agent-tab h-8 min-w-0 max-w-[168px] px-3 text-xs"
            >
              <span className="truncate">{agent.name}</span>
            </TabsTrigger>
          ))}
        </TabsList>
        <ScrollBar orientation="horizontal" />
      </ScrollArea>
    </div>
  );
}

export function trayAgentTabId(
  selectedAgentId: "overview" | string,
  agents: TrayAgentUsageView[],
): string {
  if (selectedAgentId === "overview") return "tray-agent-tab-overview";
  const index = agents.findIndex(
    (agent) => agent.agentModuleId === selectedAgentId,
  );
  return index < 0 ? "tray-agent-tab-overview" : `tray-agent-tab-${index}`;
}
