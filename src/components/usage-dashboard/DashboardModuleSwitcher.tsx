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
import type { DashboardModuleView } from "@/types/usageDashboard";

interface DashboardModuleSwitcherProps {
  modules: DashboardModuleView[];
  selectedModuleId: string;
  onSelect: (moduleId: string) => void;
}

export function DashboardModuleSwitcher({
  modules,
  selectedModuleId,
  onSelect,
}: DashboardModuleSwitcherProps) {
  const { t } = useTranslation();
  const visibleModules = useMemo(
    () =>
      [...modules]
        .filter((module) => module.visible)
        .sort((left, right) => left.sortOrder - right.sortOrder),
    [modules],
  );

  if (!visibleModules.length) return null;

  return (
    <div className="flex min-w-0 items-center justify-center gap-2">
      <div className="min-w-0 overflow-x-auto">
        <Tabs value={selectedModuleId} onValueChange={onSelect}>
          <TabsList
            aria-label={t("dashboardModules.navigation", {
              defaultValue: "Dashboard modules",
            })}
            className="w-max max-w-none justify-start"
          >
            {visibleModules.map((module) => (
              <TabsTrigger
                key={module.id}
                value={module.id}
                className="min-w-0 whitespace-nowrap"
              >
                {module.name}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      {visibleModules.length > 4 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="outline"
              aria-label={t("dashboardModules.more", {
                defaultValue: "More modules",
              })}
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {visibleModules.map((module) => (
              <DropdownMenuItem
                key={module.id}
                aria-current={
                  module.id === selectedModuleId ? "page" : undefined
                }
                onSelect={() => onSelect(module.id)}
              >
                {module.name}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}
