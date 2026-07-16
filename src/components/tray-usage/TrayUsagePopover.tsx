import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import {
  hideTrayPopover,
  openMainFromTray,
  quitFromTray,
} from "@/lib/api/trayUsage";
import {
  trayUsageKeys,
  useRefreshTrayUsage,
  useTrayUsageEventBridge,
  useTrayUsageSnapshot,
} from "@/lib/query/trayUsage";
import type { MainWindowDestination } from "@/types/trayUsage";
import { TrayUsagePopoverView } from "./TrayUsagePopoverView";

const initialRefreshKey = [...trayUsageKeys.all, "initial-refresh"] as const;

export function TrayUsagePopover() {
  const queryClient = useQueryClient();
  const { data, isLoading } = useTrayUsageSnapshot();
  const refresh = useRefreshTrayUsage();
  const [selectedAgentId, setSelectedAgentId] = useState<"overview" | string>(
    "overview",
  );
  const hiddenForCycle = useRef(false);
  const navigationInProgress = useRef(false);
  const showCycle = useRef(0);

  useTrayUsageEventBridge();

  useEffect(() => {
    if (queryClient.getQueryData(initialRefreshKey)) return;
    queryClient.setQueryData(initialRefreshKey, true);
    refresh.mutate();
  }, [queryClient, refresh.mutate]);

  useEffect(() => {
    if (
      selectedAgentId !== "overview" &&
      data &&
      !data.agents.some((agent) => agent.agentModuleId === selectedAgentId)
    ) {
      setSelectedAgentId("overview");
    }
  }, [data, selectedAgentId]);

  const requestHide = useCallback(() => {
    if (hiddenForCycle.current || navigationInProgress.current) return;
    const requestedCycle = showCycle.current;
    hiddenForCycle.current = true;
    void hideTrayPopover().catch(() => {
      if (
        showCycle.current === requestedCycle &&
        !navigationInProgress.current
      ) {
        hiddenForCycle.current = false;
      }
    });
  }, []);

  useTauriEvent("tray-popover-shown", () => {
    showCycle.current += 1;
    hiddenForCycle.current = false;
    navigationInProgress.current = false;
    if (!refresh.isPending && !data?.refreshInProgress) {
      refresh.mutate();
    }
  });

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") requestHide();
    };
    const handleBlur = () => requestHide();
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("blur", handleBlur);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("blur", handleBlur);
    };
  }, [requestHide]);

  const openDestination = (destination: MainWindowDestination) => {
    if (navigationInProgress.current || hiddenForCycle.current) return;
    const requestedCycle = showCycle.current;
    navigationInProgress.current = true;
    hiddenForCycle.current = true;
    void openMainFromTray(destination).catch(() => {
      if (showCycle.current === requestedCycle) {
        navigationInProgress.current = false;
        hiddenForCycle.current = false;
      }
    });
  };

  const openDetails = (agentModuleId: string | null) => {
    openDestination({ kind: "usage", agentModuleId });
  };

  const openSettings = (providerId: string | null) => {
    openDestination({ kind: "providerBudget", providerId });
  };

  return (
    <TrayUsagePopoverView
      snapshot={data ?? null}
      loading={isLoading}
      selectedAgentId={selectedAgentId}
      onSelectAgent={setSelectedAgentId}
      refreshing={refresh.isPending || Boolean(data?.refreshInProgress)}
      onRefresh={() => refresh.mutate()}
      onOpenDetails={openDetails}
      onOpenSettings={openSettings}
      onQuit={() => void quitFromTray().catch(() => undefined)}
    />
  );
}
