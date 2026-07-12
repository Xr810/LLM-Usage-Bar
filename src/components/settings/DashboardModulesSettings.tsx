import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "@/components/ConfirmDialog";
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
  useDashboardModules,
  useDeleteDashboardModule,
  useReorderDashboardModules,
  useSaveDashboardModule,
  useSetDashboardModuleVisibility,
} from "@/lib/query/usageDashboard";
import type {
  DashboardModuleInput,
  DashboardModuleView,
} from "@/types/usageDashboard";

interface ModuleRowProps {
  module: DashboardModuleView;
  index: number;
  count: number;
  isPending: boolean;
  onSave: (input: DashboardModuleInput) => Promise<void>;
  onMove: (index: number, offset: -1 | 1) => Promise<void>;
  onVisibility: (module: DashboardModuleView) => Promise<void>;
  onDelete: (module: DashboardModuleView) => void;
}

function ModuleRow({
  module,
  index,
  count,
  isPending,
  onSave,
  onMove,
  onVisibility,
  onDelete,
}: ModuleRowProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(module.name);

  useEffect(() => setName(module.name), [module.name]);

  const canDelete = !module.isSystem && module.kind === "subscription";
  const deleteBlocked = module.providerCount > 0;

  return (
    <div
      data-testid={`dashboard-module-${module.id}`}
      className="grid gap-3 rounded-lg border p-4 lg:grid-cols-[minmax(180px,1fr)_auto_auto] lg:items-center"
    >
      <div className="space-y-2">
        <Label>
          <span className="sr-only">
            {t("dashboardModules.nameFor", {
              name: module.name,
              defaultValue: `Name for ${module.name}`,
            })}
          </span>
          <Input
            aria-label={t("dashboardModules.nameFor", {
              name: module.name,
              defaultValue: `Name for ${module.name}`,
            })}
            value={name}
            disabled={isPending}
            onChange={(event) => setName(event.target.value)}
          />
        </Label>
        <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
          <span>
            {module.kind === "api"
              ? t("dashboardModules.apiType", { defaultValue: "API module" })
              : t("dashboardModules.subscriptionType", {
                  defaultValue: "Subscription module",
                })}
          </span>
          <span>
            {t("dashboardModules.providerCount", {
              count: module.providerCount,
              defaultValue: `${module.providerCount} ${
                module.providerCount === 1 ? "Provider" : "Providers"
              }`,
            })}
          </span>
          <span>
            {module.visible
              ? t("common.visible", { defaultValue: "Visible" })
              : t("common.hidden", { defaultValue: "Hidden" })}
          </span>
        </div>
        {canDelete && deleteBlocked ? (
          <p className="text-xs text-muted-foreground">
            {t("dashboardModules.deleteBlocked", {
              defaultValue: "Move or disable its Providers before deleting.",
            })}
          </p>
        ) : null}
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={isPending || !name.trim() || name.trim() === module.name}
          aria-label={t("dashboardModules.saveNamed", {
            name: module.name,
            defaultValue: `Save ${module.name}`,
          })}
          onClick={() =>
            void onSave({
              id: module.id,
              name: name.trim(),
              kind: module.kind,
              sortOrder: module.sortOrder,
              visible: module.visible,
            })
          }
        >
          {t("common.save", { defaultValue: "Save" })}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={isPending || index === 0}
          aria-label={t("dashboardModules.moveUp", {
            name: module.name,
            defaultValue: `Move ${module.name} up`,
          })}
          onClick={() => void onMove(index, -1)}
        >
          ↑
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={isPending || index === count - 1}
          aria-label={t("dashboardModules.moveDown", {
            name: module.name,
            defaultValue: `Move ${module.name} down`,
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
          aria-label={t("dashboardModules.toggleVisibility", {
            action: module.visible ? "Hide" : "Show",
            name: module.name,
            defaultValue: `${module.visible ? "Hide" : "Show"} ${module.name}`,
          })}
          onClick={() => void onVisibility(module)}
        >
          {module.visible
            ? t("common.hide", { defaultValue: "Hide" })
            : t("common.show", { defaultValue: "Show" })}
        </Button>
        {canDelete ? (
          <Button
            size="sm"
            variant="destructive"
            disabled={isPending || deleteBlocked}
            aria-label={t("dashboardModules.deleteNamed", {
              name: module.name,
              defaultValue: `Delete ${module.name}`,
            })}
            onClick={() => onDelete(module)}
          >
            {t("common.delete", { defaultValue: "Delete" })}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export function DashboardModulesSettings() {
  const { t } = useTranslation();
  const modulesQuery = useDashboardModules();
  const saveModule = useSaveDashboardModule();
  const reorderModules = useReorderDashboardModules();
  const setVisibility = useSetDashboardModuleVisibility();
  const deleteModule = useDeleteDashboardModule();
  const [newName, setNewName] = useState("");
  const [pendingDelete, setPendingDelete] =
    useState<DashboardModuleView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const modules = useMemo(
    () =>
      [...(modulesQuery.data ?? [])].sort((a, b) => a.sortOrder - b.sortOrder),
    [modulesQuery.data],
  );
  const isPending =
    saveModule.isPending ||
    reorderModules.isPending ||
    setVisibility.isPending ||
    deleteModule.isPending;

  const run = async (operation: () => Promise<unknown>) => {
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      throw cause;
    }
  };

  const create = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      await run(() =>
        saveModule.mutateAsync({
          id: null,
          name,
          kind: "subscription",
          sortOrder:
            modules.reduce(
              (maximum, module) => Math.max(maximum, module.sortOrder),
              0,
            ) + 1,
          visible: true,
        }),
      );
      setNewName("");
    } catch {
      // run already exposes the authoritative backend rejection.
    }
  };

  const move = async (index: number, offset: -1 | 1) => {
    const nextIndex = index + offset;
    if (nextIndex < 0 || nextIndex >= modules.length) return;
    const reordered = [...modules];
    [reordered[index], reordered[nextIndex]] = [
      reordered[nextIndex],
      reordered[index],
    ];
    try {
      await run(() =>
        reorderModules.mutateAsync(reordered.map((module) => module.id)),
      );
    } catch {
      // run already exposes the authoritative backend rejection.
    }
  };

  const confirmDelete = async () => {
    const module = pendingDelete;
    if (!module || module.isSystem || module.providerCount > 0) return;
    setPendingDelete(null);
    try {
      await run(() => deleteModule.mutateAsync(module.id));
    } catch {
      // run already exposes the authoritative backend rejection.
    }
  };

  const queryError = modulesQuery.error
    ? modulesQuery.error instanceof Error
      ? modulesQuery.error.message
      : String(modulesQuery.error)
    : null;

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("dashboardModules.settingsTitle", {
              defaultValue: "Usage modules",
            })}
          </CardTitle>
          <CardDescription>
            {t("dashboardModules.settingsDescription", {
              defaultValue:
                "Create subscription modules and control their name, order, and visibility.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-end gap-2">
            <Label className="min-w-[220px] flex-1">
              {t("dashboardModules.newName", {
                defaultValue: "New module name",
              })}
              <Input
                value={newName}
                disabled={isPending}
                onChange={(event) => setNewName(event.target.value)}
              />
            </Label>
            <Button
              disabled={isPending || !newName.trim()}
              onClick={() => void create()}
            >
              {t("dashboardModules.create", {
                defaultValue: "Create module",
              })}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("dashboardModules.navigationImpact", {
              defaultValue:
                "Reordering changes dashboard navigation only. Hiding removes a module from navigation and may move the dashboard to another visible module. Neither action changes Provider identity or historical usage.",
            })}
          </p>
        </CardContent>
      </Card>

      {queryError || error ? (
        <div role="alert" className="text-sm text-destructive">
          {queryError ?? error}
        </div>
      ) : null}

      {modulesQuery.isLoading ? (
        <div>{t("common.loading", { defaultValue: "Loading" })}</div>
      ) : null}
      <div className="space-y-3">
        {modules.map((module, index) => (
          <ModuleRow
            key={module.id}
            module={module}
            index={index}
            count={modules.length}
            isPending={isPending}
            onSave={async (input) => {
              try {
                await run(() => saveModule.mutateAsync(input));
              } catch {
                // run already exposes the authoritative backend rejection.
              }
            }}
            onMove={move}
            onVisibility={async (current) => {
              try {
                await run(() =>
                  setVisibility.mutateAsync({
                    moduleId: current.id,
                    visible: !current.visible,
                  }),
                );
              } catch {
                // run already exposes the authoritative backend rejection.
              }
            }}
            onDelete={setPendingDelete}
          />
        ))}
      </div>

      <ConfirmDialog
        isOpen={Boolean(pendingDelete)}
        title={t("dashboardModules.deleteTitle", {
          name: pendingDelete?.name ?? "",
          defaultValue: `Delete ${pendingDelete?.name ?? "module"}`,
        })}
        message={t("dashboardModules.deleteMessage", {
          name: pendingDelete?.name ?? "",
          defaultValue: `Delete the empty ${pendingDelete?.name ?? "module"} module?`,
        })}
        confirmText={t("common.delete", { defaultValue: "Delete" })}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}
