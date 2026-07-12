import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DashboardModulesSettings } from "./DashboardModulesSettings";
import { ProxyRoutingSettings } from "./ProxyRoutingSettings";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportSuccess?: () => void | Promise<void>;
  defaultTab?: string;
}

type SettingsTab = "modules" | "providers" | "proxy";

function resolveSettingsTab(tab?: string): SettingsTab {
  return tab === "providers" || tab === "proxy" || tab === "modules"
    ? tab
    : "modules";
}

export function SettingsPage({
  open,
  onOpenChange,
  defaultTab,
}: SettingsDialogProps) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<SettingsTab>(() =>
    resolveSettingsTab(defaultTab),
  );

  useEffect(() => {
    if (open) setTab(resolveSettingsTab(defaultTab));
  }, [defaultTab, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[90vh] max-w-6xl flex-col overflow-hidden">
        <DialogHeader>
          <DialogTitle>
            {t("settings.title", { defaultValue: "Settings" })}
          </DialogTitle>
          <DialogDescription>
            {t("settings.usageConfigurationDescription", {
              defaultValue:
                "Manage usage modules, Provider identities, and proxy forwarding.",
            })}
          </DialogDescription>
        </DialogHeader>

        <Tabs
          value={tab}
          onValueChange={(value) => setTab(resolveSettingsTab(value))}
          className="flex min-h-0 flex-1 flex-col px-6"
        >
          <TabsList
            aria-label={t("settings.sections", {
              defaultValue: "Settings sections",
            })}
            className="w-full flex-shrink-0 justify-start overflow-x-auto"
          >
            <TabsTrigger value="modules">
              {t("settings.usageModules", { defaultValue: "Usage modules" })}
            </TabsTrigger>
            <TabsTrigger value="providers">
              {t("settings.provider", { defaultValue: "Provider" })}
            </TabsTrigger>
            <TabsTrigger value="proxy">
              {t("settings.proxyRouting", {
                defaultValue: "Proxy & routing",
              })}
            </TabsTrigger>
          </TabsList>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <TabsContent value="modules">
              <DashboardModulesSettings />
            </TabsContent>
            <TabsContent value="providers">
              <UsageProvidersSettings />
            </TabsContent>
            <TabsContent value="proxy">
              <ProxyRoutingSettings />
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
