import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { AgentModuleView } from "@/types/usageDashboard";
import {
  agentModuleStorageKey,
  useAgentModuleSelection,
} from "./useDashboardModuleSelection";

function agent(
  id: string,
  sortOrder: number,
  overrides: Partial<AgentModuleView> = {},
): AgentModuleView {
  return {
    id,
    name: `Display ${id}`,
    sortOrder,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 0,
    ...overrides,
  };
}

describe("useAgentModuleSelection", () => {
  beforeEach(() => localStorage.clear());

  it("selects the first visible Agent and persists stable IDs", async () => {
    const agents = [
      agent("claude-code", 2),
      agent("hidden", 0, { visible: false }),
      agent("codex", 1),
    ];
    const { result } = renderHook(() => useAgentModuleSelection(agents));

    expect(result.current.selectedAgent?.id).toBe("codex");
    await waitFor(() =>
      expect(localStorage.getItem(agentModuleStorageKey)).toBe("codex"),
    );

    act(() => result.current.selectAgent("claude-code"));
    expect(result.current.selectedAgent?.id).toBe("claude-code");
    expect(localStorage.getItem(agentModuleStorageKey)).toBe("claude-code");
  });

  it("restores a visible persisted Agent", () => {
    localStorage.setItem(agentModuleStorageKey, "claude-code");
    const { result } = renderHook(() =>
      useAgentModuleSelection([agent("codex", 1), agent("claude-code", 2)]),
    );

    expect(result.current.selectedAgent?.id).toBe("claude-code");
  });

  it("preserves a persisted Agent while the query is initially empty", async () => {
    localStorage.setItem(agentModuleStorageKey, "claude-code");
    const { result, rerender } = renderHook(
      ({ agents }: { agents: AgentModuleView[] }) =>
        useAgentModuleSelection(agents),
      { initialProps: { agents: [] as AgentModuleView[] } },
    );

    expect(result.current.selectedAgent).toBeNull();
    await waitFor(() =>
      expect(localStorage.getItem(agentModuleStorageKey)).toBe("claude-code"),
    );

    rerender({
      agents: [agent("codex", 1), agent("claude-code", 2)],
    });
    expect(result.current.selectedAgent?.id).toBe("claude-code");
  });

  it("falls back after the selected Agent is hidden or deleted", async () => {
    localStorage.setItem(agentModuleStorageKey, "claude-code");
    const { result, rerender } = renderHook(
      ({ agents }: { agents: AgentModuleView[] }) =>
        useAgentModuleSelection(agents),
      {
        initialProps: {
          agents: [agent("codex", 1), agent("claude-code", 2)],
        },
      },
    );
    expect(result.current.selectedAgent?.id).toBe("claude-code");

    rerender({
      agents: [agent("codex", 1), agent("claude-code", 2, { visible: false })],
    });

    expect(result.current.selectedAgent?.id).toBe("codex");
    await waitFor(() =>
      expect(localStorage.getItem(agentModuleStorageKey)).toBe("codex"),
    );
  });

  it("ignores selection attempts for hidden or unknown Agents", () => {
    const { result } = renderHook(() =>
      useAgentModuleSelection([
        agent("codex", 1),
        agent("hidden", 2, { visible: false }),
      ]),
    );

    act(() => result.current.selectAgent("hidden"));
    act(() => result.current.selectAgent("api"));
    expect(result.current.selectedAgent?.id).toBe("codex");
  });

  it("excludes archived Agents even when a stale view marks them visible", () => {
    localStorage.setItem(agentModuleStorageKey, "archived-agent");
    const { result } = renderHook(() =>
      useAgentModuleSelection([
        agent("codex", 1),
        agent("archived-agent", 0, { archivedAt: 1_000 }),
      ]),
    );

    expect(result.current.visibleAgents.map((item) => item.id)).toEqual([
      "codex",
    ]);
    expect(result.current.selectedAgent?.id).toBe("codex");
  });

  it("returns null and clears persistence when no Agent is visible", async () => {
    localStorage.setItem(agentModuleStorageKey, "hidden");
    const { result } = renderHook(() =>
      useAgentModuleSelection([agent("hidden", 1, { visible: false })]),
    );

    expect(result.current.selectedAgent).toBeNull();
    await waitFor(() =>
      expect(localStorage.getItem(agentModuleStorageKey)).toBeNull(),
    );
  });
});
