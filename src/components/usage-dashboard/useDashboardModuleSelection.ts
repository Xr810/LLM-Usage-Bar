import { useCallback, useEffect, useMemo, useState } from "react";
import type { AgentModuleView } from "@/types/usageDashboard";

export const agentModuleStorageKey = "llm-usage-bar:last-agent-module-id";

function readStoredAgentId(): string | null {
  if (typeof window === "undefined") return null;
  return window.localStorage.getItem(agentModuleStorageKey);
}

export function useAgentModuleSelection(agents: AgentModuleView[]) {
  const visibleAgents = useMemo(
    () =>
      [...agents]
        .filter((agent) => agent.visible && agent.archivedAt == null)
        .sort((left, right) => left.sortOrder - right.sortOrder),
    [agents],
  );
  const [selectedId, setSelectedId] = useState<string | null>(
    readStoredAgentId,
  );
  const selectedAgent =
    visibleAgents.find((agent) => agent.id === selectedId) ??
    visibleAgents[0] ??
    null;

  useEffect(() => {
    // React Query exposes an empty array before the first Agent response. Keep the
    // persisted ID through that transient state so the later result can restore it.
    if (agents.length === 0) return;
    const nextId = selectedAgent?.id ?? null;
    if (nextId !== selectedId) setSelectedId(nextId);
    if (typeof window === "undefined") return;
    if (nextId) {
      window.localStorage.setItem(agentModuleStorageKey, nextId);
    } else {
      window.localStorage.removeItem(agentModuleStorageKey);
    }
  }, [agents.length, selectedAgent?.id, selectedId]);

  const selectAgent = useCallback(
    (agentModuleId: string) => {
      if (!visibleAgents.some((agent) => agent.id === agentModuleId)) return;
      setSelectedId(agentModuleId);
      if (typeof window !== "undefined") {
        window.localStorage.setItem(agentModuleStorageKey, agentModuleId);
      }
    },
    [visibleAgents],
  );

  return { selectedAgent, selectAgent, visibleAgents };
}
