import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentsSettings } from "./DashboardModulesSettings";

type DragEndHandler = (event: {
  active: { id: string };
  over: { id: string } | null;
}) => void;

const dndMocks = vi.hoisted(() => ({
  onDragEnd: undefined as DragEndHandler | undefined,
  onKeyDown: vi.fn(),
}));

vi.mock("@dnd-kit/core", () => ({
  DndContext: ({
    children,
    onDragEnd,
  }: {
    children: ReactNode;
    onDragEnd: DragEndHandler;
  }) => {
    dndMocks.onDragEnd = onDragEnd;
    return <div data-testid="agent-dnd-context">{children}</div>;
  },
  closestCenter: vi.fn(),
  KeyboardSensor: class KeyboardSensor {},
  PointerSensor: class PointerSensor {},
  useSensor: vi.fn(() => ({})),
  useSensors: vi.fn((...sensors: unknown[]) => sensors),
}));

vi.mock("@dnd-kit/sortable", () => ({
  SortableContext: ({ children }: { children: ReactNode }) => <>{children}</>,
  arrayMove: <T,>(items: T[], oldIndex: number, newIndex: number) => {
    const reordered = [...items];
    const [item] = reordered.splice(oldIndex, 1);
    reordered.splice(newIndex, 0, item);
    return reordered;
  },
  horizontalListSortingStrategy: vi.fn(),
  sortableKeyboardCoordinates: vi.fn(),
  useSortable: ({ id }: { id: string }) => ({
    setNodeRef: vi.fn(),
    setActivatorNodeRef: vi.fn(),
    attributes: { "data-sortable-id": id },
    listeners: { onKeyDown: dndMocks.onKeyDown },
    transform: null,
    transition: undefined,
    isDragging: false,
  }),
}));

const mocks = vi.hoisted(() => ({
  agents: [] as Array<Record<string, unknown>>,
  providers: [] as Array<Record<string, unknown>>,
  bindings: [] as Array<Record<string, unknown>>,
  saveAgent: vi.fn(),
  reorderAgents: vi.fn(),
  setVisibility: vi.fn(),
  deleteAgent: vi.fn(),
  saveBinding: vi.fn(),
  deleteBinding: vi.fn(),
  setKey: vi.fn(),
  replaceKey: vi.fn(),
  clearKey: vi.fn(),
}));

vi.mock("@/lib/query/usageDashboard", () => ({
  useAgentModules: () => ({
    data: mocks.agents,
    isLoading: false,
    error: null,
  }),
  useUsageProviders: () => ({
    data: mocks.providers,
    isLoading: false,
    error: null,
  }),
  useAgentProviderBindings: () => ({
    data: mocks.bindings,
    isLoading: false,
    error: null,
  }),
  useSaveAgentModule: () => ({
    mutateAsync: mocks.saveAgent,
    isPending: false,
  }),
  useReorderAgentModules: () => ({
    mutateAsync: mocks.reorderAgents,
    isPending: false,
  }),
  useSetAgentModuleVisibility: () => ({
    mutateAsync: mocks.setVisibility,
    isPending: false,
  }),
  useDeleteAgentModule: () => ({
    mutateAsync: mocks.deleteAgent,
    isPending: false,
  }),
  useSaveAgentProviderBinding: () => ({
    mutateAsync: mocks.saveBinding,
    isPending: false,
  }),
  useDeleteAgentProviderBinding: () => ({
    mutateAsync: mocks.deleteBinding,
    isPending: false,
  }),
  useAgentProviderBindingCredentialActions: () => ({
    setApiKey: mocks.setKey,
    replaceApiKey: mocks.replaceKey,
    clearApiKey: mocks.clearKey,
    isPending: false,
  }),
}));

const agentFixtures = [
  {
    id: "codex",
    name: "Codex",
    sortOrder: 1,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 1,
  },
  {
    id: "claude-code",
    name: "Claude Code",
    sortOrder: 2,
    visible: true,
    isFixed: true,
    archivedAt: null,
    providerCount: 0,
  },
  {
    id: "custom-research",
    name: "Research Agent",
    sortOrder: 3,
    visible: true,
    isFixed: false,
    archivedAt: null,
    providerCount: 2,
  },
];

const providerFixtures = [
  { id: "provider-a", name: "Provider A", enabled: true },
  { id: "provider-b", name: "Provider B", enabled: true },
];

const bindingFixtures = [
  {
    id: "binding-a",
    agentModuleId: "codex",
    providerId: "provider-a",
    enabled: true,
    effectiveEnabled: false,
    credentialStatus: "configured",
    canClearCredential: true,
    credentialVersion: 3,
    createdAt: 1,
    updatedAt: 1,
  },
];

describe("AgentsSettings", () => {
  beforeEach(() => {
    mocks.agents = agentFixtures.map((agent) => ({ ...agent }));
    mocks.providers = providerFixtures.map((provider) => ({ ...provider }));
    mocks.bindings = bindingFixtures.map((binding) => ({ ...binding }));
    for (const mock of [
      mocks.saveAgent,
      mocks.reorderAgents,
      mocks.setVisibility,
      mocks.deleteAgent,
      mocks.saveBinding,
      mocks.deleteBinding,
      mocks.setKey,
      mocks.replaceKey,
      mocks.clearKey,
    ]) {
      mock.mockReset().mockResolvedValue(undefined);
    }
    dndMocks.onDragEnd = undefined;
    dndMocks.onKeyDown.mockReset();
  });

  it("shows Agents in one compact row without arrow reorder controls", async () => {
    render(<AgentsSettings />);

    const sorter = screen.getByTestId("agent-sorter");
    expect(sorter).toHaveClass("flex-nowrap");
    expect(within(sorter).getAllByTestId(/^agent-sort-/)).toHaveLength(3);
    expect(within(sorter).getByText("Codex")).toBeInTheDocument();
    expect(within(sorter).getByText("Claude Code")).toBeInTheDocument();
    expect(within(sorter).getByText("Research Agent")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Move Codex up" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Move Codex down" }),
    ).toBeNull();

    const fixed = screen.getByTestId("agent-settings-codex");
    expect(within(fixed).getByText("Fixed Agent")).toBeInTheDocument();
    expect(within(fixed).queryByLabelText("Name for Codex")).toBeNull();
    expect(
      within(fixed).queryByRole("button", { name: "Delete Codex" }),
    ).toBeNull();

    fireEvent.click(within(fixed).getByRole("button", { name: "Hide Codex" }));
    await waitFor(() =>
      expect(mocks.setVisibility).toHaveBeenCalledWith({
        agentModuleId: "codex",
        visible: false,
      }),
    );
  });

  it("persists the navigation order after an Agent is dragged", async () => {
    render(<AgentsSettings />);

    expect(dndMocks.onDragEnd).toBeTypeOf("function");
    dndMocks.onDragEnd?.({
      active: { id: "codex" },
      over: { id: "claude-code" },
    });

    await waitFor(() =>
      expect(mocks.reorderAgents).toHaveBeenCalledWith([
        "claude-code",
        "codex",
        "custom-research",
      ]),
    );
  });

  it("keeps selection separate from the accessible drag handle", () => {
    render(<AgentsSettings />);

    const selectionButton = screen.getByTestId("agent-sort-codex");
    const dragHandle = screen.getByRole("button", { name: "Reorder Codex" });

    expect(selectionButton).not.toHaveAttribute("data-sortable-id");
    expect(dragHandle).toHaveAttribute("data-sortable-id", "codex");

    fireEvent.keyDown(selectionButton, { key: "Enter" });
    expect(dndMocks.onKeyDown).not.toHaveBeenCalled();

    fireEvent.keyDown(dragHandle, { key: "Enter" });
    expect(dndMocks.onKeyDown).toHaveBeenCalledTimes(1);
  });

  it("explicitly creates, renames, and deletes Custom Agents regardless of Provider count", async () => {
    render(<AgentsSettings />);

    fireEvent.change(screen.getByLabelText("Custom Agent name"), {
      target: { value: "Sidekick" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Create Custom Agent" }),
    );
    await waitFor(() =>
      expect(mocks.saveAgent).toHaveBeenCalledWith({
        id: null,
        name: "Sidekick",
        sortOrder: 4,
        visible: true,
      }),
    );

    fireEvent.click(screen.getByTestId("agent-sort-custom-research"));
    const custom = screen.getByTestId("agent-settings-custom-research");
    fireEvent.change(within(custom).getByLabelText("Name for Research Agent"), {
      target: { value: "Research Team" },
    });
    fireEvent.click(
      within(custom).getByRole("button", { name: "Save Research Agent" }),
    );
    await waitFor(() =>
      expect(mocks.saveAgent).toHaveBeenCalledWith({
        id: "custom-research",
        name: "Research Team",
        sortOrder: 3,
        visible: true,
      }),
    );

    const deleteButton = within(custom).getByRole("button", {
      name: "Delete Research Agent",
    });
    expect(deleteButton).toBeEnabled();
    fireEvent.click(deleteButton);
    await waitFor(() =>
      expect(mocks.deleteAgent).toHaveBeenCalledWith("custom-research"),
    );
  });

  it("adds bindings disabled, saves complete stable toggles, and deletes by credential version", async () => {
    render(<AgentsSettings />);
    const fixed = screen.getByTestId("agent-settings-codex");

    expect(within(fixed).getByText("Requested: Enabled")).toBeInTheDocument();
    expect(within(fixed).getByText("Effective: Disabled")).toBeInTheDocument();

    fireEvent.click(
      within(fixed).getByRole("button", { name: "Disable Provider A" }),
    );
    await waitFor(() =>
      expect(mocks.saveBinding).toHaveBeenCalledWith({
        id: "binding-a",
        agentModuleId: "codex",
        providerId: "provider-a",
        enabled: false,
      }),
    );

    fireEvent.change(within(fixed).getByLabelText("Add Provider for Codex"), {
      target: { value: "provider-b" },
    });
    fireEvent.click(
      within(fixed).getByRole("button", { name: "Add binding for Codex" }),
    );
    await waitFor(() =>
      expect(mocks.saveBinding).toHaveBeenCalledWith({
        id: null,
        agentModuleId: "codex",
        providerId: "provider-b",
        enabled: false,
      }),
    );

    fireEvent.click(
      within(fixed).getByRole("button", { name: "Delete Provider A binding" }),
    );
    await waitFor(() =>
      expect(mocks.deleteBinding).toHaveBeenCalledWith({
        bindingId: "binding-a",
        expectedVersion: 3,
      }),
    );
  });

  it("opens only the allowed public credential actions", () => {
    render(<AgentsSettings />);
    const binding = screen.getByTestId("agent-binding-binding-a");

    expect(within(binding).getByText("Configured")).toBeInTheDocument();
    expect(
      within(binding).getByRole("button", { name: "Replace API key" }),
    ).toBeInTheDocument();
    expect(
      within(binding).getByRole("button", { name: "Clear API key" }),
    ).toBeInTheDocument();
    expect(within(binding).queryByText(/\*\*|••/)).toBeNull();
  });

  it("allows an unavailable protected credential to be cleared", async () => {
    mocks.bindings = bindingFixtures.map((binding) => ({
      ...binding,
      credentialStatus: "unavailable",
      canClearCredential: true,
      credentialVersion: 7,
    }));
    render(<AgentsSettings />);

    const binding = screen.getByTestId("agent-binding-binding-a");
    expect(within(binding).getByText("Unavailable")).toBeInTheDocument();
    expect(
      within(binding).queryByRole("button", { name: "Replace API key" }),
    ).toBeNull();
    fireEvent.click(
      within(binding).getByRole("button", { name: "Clear API key" }),
    );
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Clear API key",
      }),
    );

    await waitFor(() =>
      expect(mocks.clearKey).toHaveBeenCalledWith("binding-a", 7),
    );
  });

  it("offers no key action when an unavailable credential cannot be cleared", () => {
    mocks.bindings = bindingFixtures.map((binding) => ({
      ...binding,
      credentialStatus: "unavailable",
      canClearCredential: false,
      credentialVersion: 7,
    }));
    render(<AgentsSettings />);

    const binding = screen.getByTestId("agent-binding-binding-a");
    expect(within(binding).getByText("Unavailable")).toBeInTheDocument();
    expect(
      within(binding).queryByRole("button", { name: "Clear API key" }),
    ).toBeNull();
    expect(
      within(binding).queryByRole("button", { name: "Replace API key" }),
    ).toBeNull();
  });
});
