import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { UsageDiagnosticsPanel } from "./UsageDiagnosticsPanel";
import { UsageProvidersSettings } from "./UsageProvidersSettings";
import { StartupSettings } from "./StartupSettings";
import { UsageThresholdSettings } from "./UsageThresholdSettings";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportSuccess?: () => void | Promise<void>;
  defaultTab?: string;
  defaultProviderId?: string;
  onProviderTargetHandled?: () => void;
}

type SettingsTab = "general" | "providers" | "diagnostics";

function resolveSettingsTab(tab?: string): SettingsTab {
  if (tab === "providers" || tab === "diagnostics") {
    return tab;
  }
  return "general";
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
        <DialogHeader className="relative pr-14">
          <DialogClose
            className="absolute right-5 top-5 rounded-full p-1.5 transition-colors hover:bg-muted focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2"
            aria-label={t("common.close", { defaultValue: "Close" })}
          >
            <X className="size-4 text-muted-foreground" />
          </DialogClose>
          <DialogTitle>
            {t("settings.title", { defaultValue: "Settings" })}
          </DialogTitle>
          <DialogDescription>
            {t("settings.usageConfigurationDescription", {
              defaultValue:
                "Manage Provider accounts, monitoring sources, budgets, and diagnostics.",
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
            <TabsTrigger value="general">
              {t("settings.general", { defaultValue: "General" })}
            </TabsTrigger>
            <TabsTrigger value="providers">
              {t("settings.providers", { defaultValue: "Providers" })}
            </TabsTrigger>
            <TabsTrigger value="diagnostics">
              {t("settings.diagnostics", { defaultValue: "Diagnostics" })}
            </TabsTrigger>
          </TabsList>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <TabsContent value="general">
              <StartupSettings />
              <UsageThresholdSettings />
            </TabsContent>
            <TabsContent value="providers">
              <UsageProvidersSettings
                targetProviderId={targetProviderId ?? undefined}
                onTargetHandled={handleProviderTargetHandled}
              />
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
