import { useCallback, useEffect, useMemo, useState } from "react";
import type { DashboardModuleView } from "@/types/usageDashboard";

export const dashboardModuleStorageKey =
  "llm-usage-bar:last-dashboard-module-id";

function readStoredModuleId(): string | null {
  if (typeof window === "undefined") return null;
  return window.localStorage.getItem(dashboardModuleStorageKey);
}

export function useDashboardModuleSelection(modules: DashboardModuleView[]) {
  const visibleModules = useMemo(
    () =>
      [...modules]
        .filter((module) => module.visible)
        .sort((left, right) => left.sortOrder - right.sortOrder),
    [modules],
  );
  const [selectedId, setSelectedId] = useState<string | null>(
    readStoredModuleId,
  );
  const selectedModule =
    visibleModules.find((module) => module.id === selectedId) ??
    visibleModules[0] ??
    null;

  useEffect(() => {
    const nextId = selectedModule?.id ?? null;
    if (nextId !== selectedId) setSelectedId(nextId);
    if (typeof window === "undefined") return;
    if (nextId) {
      window.localStorage.setItem(dashboardModuleStorageKey, nextId);
    } else {
      window.localStorage.removeItem(dashboardModuleStorageKey);
    }
  }, [selectedId, selectedModule?.id]);

  const selectModule = useCallback(
    (moduleId: string) => {
      if (!visibleModules.some((module) => module.id === moduleId)) return;
      setSelectedId(moduleId);
      if (typeof window !== "undefined") {
        window.localStorage.setItem(dashboardModuleStorageKey, moduleId);
      }
    },
    [visibleModules],
  );

  return { selectedModule, selectModule, visibleModules };
}
