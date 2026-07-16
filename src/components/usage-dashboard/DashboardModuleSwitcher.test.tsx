import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AgentModuleView } from "@/types/usageDashboard";
import { AgentSwitcher } from "./DashboardModuleSwitcher";

function agent(
  id: string,
  name: string,
  sortOrder: number,
  visible = true,
  isFixed = true,
): AgentModuleView {
  return {
    id,
    name,
    sortOrder,
    visible,
    isFixed,
    archivedAt: null,
    providerCount: 0,
  };
}

describe("AgentSwitcher", () => {
  it("renders visible Agents in backend order without a synthetic API Agent", () => {
    const archivedAgent = agent(
      "archived-agent",
      "Archived Agent",
      0,
      true,
      false,
    );
    archivedAgent.archivedAt = 1_000;
    render(
      <AgentSwitcher
        agents={[
          archivedAgent,
          agent("hermes", "Hermes", 5),
          agent("claude-code", "Claude Code", 2),
          agent("hidden", "Hidden", 0, false, false),
          agent("codex", "Codex", 1),
          agent("opencode", "OpenCode", 3),
          agent("openclaw", "OpenClaw", 4),
          agent("custom-research", "Research Agent", 6, true, false),
        ]}
        selectedAgentId="codex"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Codex",
      "Claude Code",
      "OpenCode",
      "OpenClaw",
      "Hermes",
      "Research Agent",
    ]);
    expect(screen.queryByRole("tab", { name: "Hidden" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Archived Agent" })).toBeNull();
    expect(screen.queryByRole("tab", { name: /API/i })).toBeNull();
    expect(screen.getByRole("tab", { name: "Codex" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tablist")).toHaveAccessibleName("Agents");
  });

  it("supports keyboard and direct selection by stable Agent ID", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    render(
      <AgentSwitcher
        agents={[
          agent("codex", "Codex", 1),
          agent("claude-code", "Claude Code", 2),
          agent("opencode", "OpenCode", 3),
        ]}
        selectedAgentId="codex"
        onSelect={onSelect}
      />,
    );

    await user.click(screen.getByRole("tab", { name: "Codex" }));
    await user.keyboard("{ArrowRight}");
    expect(onSelect).toHaveBeenCalledWith("claude-code");

    await user.click(screen.getByRole("tab", { name: "OpenCode" }));
    expect(onSelect).toHaveBeenCalledWith("opencode");
  });

  it("does not render a redundant More menu when every Agent remains directly selectable", () => {
    render(
      <AgentSwitcher
        agents={Array.from({ length: 6 }, (_, index) =>
          agent(`agent-${index + 1}`, `Agent ${index + 1}`, index + 1),
        )}
        selectedAgentId="agent-1"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getAllByRole("tab")).toHaveLength(6);
    expect(screen.queryByRole("button", { name: "More Agents" })).toBeNull();
  });
});
