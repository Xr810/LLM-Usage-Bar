import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Activity, Blocks, Settings2, X, type LucideIcon } from "lucide-react";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DRAG_REGION_ATTR, DRAG_REGION_STYLE, isMac } from "@/lib/platform";
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

interface SettingsTabMeta {
  value: SettingsTab;
  labelKey: string;
  labelDefault: string;
  icon: LucideIcon;
}

const SETTINGS_TABS: SettingsTabMeta[] = [
  {
    value: "general",
    labelKey: "settings.general",
    labelDefault: "General",
    icon: Settings2,
  },
  {
    value: "providers",
    labelKey: "settings.providers",
    labelDefault: "Providers",
    icon: Blocks,
  },
  {
    value: "diagnostics",
    labelKey: "settings.diagnostics",
    labelDefault: "Diagnostics",
    icon: Activity,
  },
];

const NO_DRAG_STYLE = { WebkitAppRegion: "no-drag" } as CSSProperties;

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

  const activeTabMeta =
    SETTINGS_TABS.find((item) => item.value === tab) ?? SETTINGS_TABS[0];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        variant="fullscreen"
        overlayClassName="hidden"
        className="overflow-hidden"
      >
        <Tabs
          value={tab}
          onValueChange={(value) => setTab(resolveSettingsTab(value))}
          orientation="vertical"
          className="flex min-h-0 w-full flex-1"
        >
          {/* 左侧导航栏（macOS 系统设置风格） */}
          <aside
            data-settings-region="sidebar"
            className="flex w-60 shrink-0 flex-col border-r border-border/60 bg-muted/30"
          >
            {isMac() ? (
              <div
                aria-hidden="true"
                className="h-11 shrink-0"
                {...DRAG_REGION_ATTR}
                style={DRAG_REGION_STYLE as CSSProperties}
              />
            ) : null}
            <div className="px-5 pb-5 pt-1">
              <DialogTitle className="text-lg font-semibold tracking-tight">
                {t("settings.title", { defaultValue: "Settings" })}
              </DialogTitle>
              <DialogDescription className="mt-1.5 text-xs leading-relaxed text-muted-foreground">
                {t("settings.usageConfigurationDescription", {
                  defaultValue:
                    "Manage Provider accounts, monitoring sources, budgets, and diagnostics.",
                })}
              </DialogDescription>
            </div>
            <TabsList
              aria-label={t("settings.sections", {
                defaultValue: "Settings sections",
              })}
              className="flex w-full flex-1 flex-col items-stretch justify-start gap-1 overflow-y-auto rounded-none bg-transparent p-0 px-3 pb-4"
            >
              {SETTINGS_TABS.map((item) => {
                const Icon = item.icon;
                return (
                  <TabsTrigger
                    key={item.value}
                    value={item.value}
                    className="min-w-0 justify-start gap-2.5 rounded-lg px-3 py-2 text-[13px] font-medium text-muted-foreground transition-colors data-[state=active]:bg-primary/10 data-[state=active]:text-primary data-[state=active]:shadow-none data-[state=inactive]:opacity-100 data-[state=inactive]:hover:bg-muted data-[state=inactive]:hover:text-foreground"
                  >
                    <Icon className="h-4 w-4 shrink-0" aria-hidden="true" />
                    <span className="truncate">
                      {t(item.labelKey, { defaultValue: item.labelDefault })}
                    </span>
                  </TabsTrigger>
                );
              })}
            </TabsList>
          </aside>

          {/* 右侧内容区 */}
          <div
            data-settings-region="pane"
            className="flex min-w-0 flex-1 flex-col bg-background"
          >
            <header
              data-settings-region="header"
              className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border/60 pl-7 pr-5"
              {...DRAG_REGION_ATTR}
              style={DRAG_REGION_STYLE as CSSProperties}
            >
              <h2 className="truncate text-[15px] font-semibold tracking-tight">
                {t(activeTabMeta.labelKey, {
                  defaultValue: activeTabMeta.labelDefault,
                })}
              </h2>
              <DialogClose
                className="rounded-full p-1.5 transition-colors hover:bg-muted focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2"
                style={NO_DRAG_STYLE}
                aria-label={t("common.close", { defaultValue: "Close" })}
              >
                <X className="size-4 text-muted-foreground" />
              </DialogClose>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto w-full max-w-3xl px-7 pb-10 pt-5">
                <TabsContent value="general" className="mt-0">
                  <StartupSettings />
                  <UsageThresholdSettings />
                </TabsContent>
                <TabsContent value="providers" className="mt-0">
                  <UsageProvidersSettings
                    targetProviderId={targetProviderId ?? undefined}
                    onTargetHandled={handleProviderTargetHandled}
                  />
                </TabsContent>
                <TabsContent value="diagnostics" className="mt-0">
                  <UsageDiagnosticsPanel />
                </TabsContent>
              </div>
            </div>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
