import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type {
  DashboardModuleView,
  ProviderUsageView,
} from "@/types/usageDashboard";
import { SubscriptionProviderCard } from "./SubscriptionProviderCard";

interface SubscriptionModulePageProps {
  module: DashboardModuleView;
  providers: ProviderUsageView[];
  onRefreshQuota: (providerId: string) => Promise<unknown>;
  onSyncSessions: (providerId: string) => Promise<void>;
  onOpenSettings?: () => void;
  isRefreshingQuota?: boolean;
  isSyncingSessions?: boolean;
}

export function SubscriptionModulePage({
  module,
  providers,
  onRefreshQuota,
  onSyncSessions,
  onOpenSettings,
  isRefreshingQuota = false,
  isSyncingSessions = false,
}: SubscriptionModulePageProps) {
  const { t } = useTranslation();
  const [selectedProviderId, setSelectedProviderId] = useState(
    () => providers[0]?.provider.id ?? "",
  );
  const selectedProvider =
    providers.find((usage) => usage.provider.id === selectedProviderId) ??
    providers[0] ??
    null;

  useEffect(() => {
    const nextId = selectedProvider?.provider.id ?? "";
    if (nextId !== selectedProviderId) setSelectedProviderId(nextId);
  }, [selectedProvider?.provider.id, selectedProviderId]);

  if (!selectedProvider) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center">
        <p className="text-sm text-muted-foreground">
          {t("usageDashboard.subscriptionModuleEmpty", {
            defaultValue:
              "No subscription account has been added to this module.",
          })}
        </p>
        {onOpenSettings ? (
          <Button className="mt-3" size="sm" onClick={onOpenSettings}>
            {t("usageDashboard.addProviderInSettings", {
              defaultValue: "Add Provider in Settings",
            })}
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {providers.length > 1 ? (
        <Tabs
          value={selectedProvider.provider.id}
          onValueChange={setSelectedProviderId}
        >
          <div className="overflow-x-auto">
            <TabsList
              aria-label={t("usageDashboard.providersInModule", {
                module: module.name,
                defaultValue: `Providers in ${module.name}`,
              })}
              className="w-max justify-start"
            >
              {providers.map((usage) => (
                <TabsTrigger
                  key={usage.provider.id}
                  value={usage.provider.id}
                  className="min-w-0"
                >
                  {usage.provider.name}
                </TabsTrigger>
              ))}
            </TabsList>
          </div>
          {providers.map((usage) => (
            <TabsContent key={usage.provider.id} value={usage.provider.id}>
              <SubscriptionProviderCard
                usage={usage}
                onRefreshQuota={onRefreshQuota}
                onSyncSessions={onSyncSessions}
                isRefreshingQuota={isRefreshingQuota}
                isSyncingSessions={isSyncingSessions}
              />
            </TabsContent>
          ))}
        </Tabs>
      ) : (
        <SubscriptionProviderCard
          usage={selectedProvider}
          onRefreshQuota={onRefreshQuota}
          onSyncSessions={onSyncSessions}
          isRefreshingQuota={isRefreshingQuota}
          isSyncingSessions={isSyncingSessions}
        />
      )}
    </div>
  );
}
