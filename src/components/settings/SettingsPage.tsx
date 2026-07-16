import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { AgentsSettings } from "./DashboardModulesSettings";
import { ProxyRoutingSettings } from "./ProxyRoutingSettings";
import { UsageDiagnosticsPanel } from "./UsageDiagnosticsPanel";
import { UsageProvidersSettings } from "./UsageProvidersSettings";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportSuccess?: () => void | Promise<void>;
  defaultTab?: string;
  defaultProviderId?: string;
  onProviderTargetHandled?: () => void;
}

type SettingsTab = "agents" | "providers" | "proxy" | "diagnostics";

function resolveSettingsTab(tab?: string): SettingsTab {
  if (tab === "providers" || tab === "proxy" || tab === "diagnostics") {
    return tab;
  }
  return "agents";
}

export function SettingsPage({
  open,
  onOpenChange,
  defaultTab,
  defaultProviderId,
  onProviderTargetHandled,
}: SettingsDialogProps) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<SettingsTab>(() =>
    resolveSettingsTab(defaultTab),
  );
  const [targetProviderId, setTargetProviderId] = useState<string | null>(
    defaultProviderId ?? null,
  );

  useEffect(() => {
    if (!open) return;
    setTab(resolveSettingsTab(defaultTab));
    setTargetProviderId(defaultProviderId ?? null);
  }, [defaultProviderId, defaultTab, open]);

  const handleProviderTargetHandled = useCallback(() => {
    setTargetProviderId(null);
    onProviderTargetHandled?.();
  }, [onProviderTargetHandled]);

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
                "Manage Agents, Provider identities, local proxy setup, and usage diagnostics.",
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
            <TabsTrigger value="agents">
              {t("settings.agents", { defaultValue: "Agents" })}
            </TabsTrigger>
            <TabsTrigger value="providers">
              {t("settings.providers", { defaultValue: "Providers" })}
            </TabsTrigger>
            <TabsTrigger value="proxy">
              {t("settings.proxySetup", {
                defaultValue: "Proxy setup",
              })}
            </TabsTrigger>
            <TabsTrigger value="diagnostics">
              {t("settings.diagnostics", { defaultValue: "Diagnostics" })}
            </TabsTrigger>
          </TabsList>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <TabsContent value="agents">
              <AgentsSettings />
            </TabsContent>
            <TabsContent value="providers">
              <UsageProvidersSettings
                targetProviderId={targetProviderId ?? undefined}
                onTargetHandled={handleProviderTargetHandled}
              />
            </TabsContent>
            <TabsContent value="proxy">
              <ProxyRoutingSettings />
            </TabsContent>
            <TabsContent value="diagnostics">
              <UsageDiagnosticsPanel />
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
