import { useCallback, useEffect, useRef, useState } from "react";
import { takePendingMainWindowDestination } from "@/lib/api/trayUsage";
import type { MainWindowDestination } from "@/types/trayUsage";
import { useTauriEvent } from "./useTauriEvent";

interface MainWindowNavigationOptions {
  /** Deprecated Agent list is ignored by Provider-only navigation. */
  agents?: unknown[];
  openUsage: (agentModuleId?: string | null) => void;
  openProviderSettings: (providerId: string | null) => void;
}

export function useMainWindowNavigation({
  openUsage,
  openProviderSettings,
}: MainWindowNavigationOptions): void {
  const [pending, setPending] = useState<MainWindowDestination[]>([]);
  const activeRef = useRef(true);
  const drainChainRef = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    activeRef.current = true;
    return () => {
      activeRef.current = false;
    };
  }, []);

  const drainPending = useCallback(() => {
    drainChainRef.current = drainChainRef.current.then(async () => {
      try {
        const destination = await takePendingMainWindowDestination();
        if (activeRef.current && destination) {
          setPending((current) => [...current, destination]);
        }
      } catch {
        console.error("Failed to consume the pending main-window destination");
      }
    });
  }, []);

  useTauriEvent<void>("main-window-navigate", drainPending);

  useEffect(() => {
    drainPending();
  }, [drainPending]);

  useEffect(() => {
    const actionableIndex = pending.findIndex(() => true);
    const destination = pending[actionableIndex];
    if (!destination) return;

    if (destination.kind === "usage") {
      openUsage();
    } else {
      openProviderSettings(destination.providerId);
    }

    setPending((current) => {
      const index = current.indexOf(destination);
      if (index < 0) return current;
      return [...current.slice(0, index), ...current.slice(index + 1)];
    });
  }, [openProviderSettings, openUsage, pending]);
}
