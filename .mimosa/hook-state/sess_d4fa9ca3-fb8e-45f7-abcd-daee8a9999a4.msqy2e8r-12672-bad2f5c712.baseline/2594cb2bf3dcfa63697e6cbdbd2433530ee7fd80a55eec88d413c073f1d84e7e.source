import { useCallback, useEffect, useRef, useState } from "react";
import {
  acknowledgeMainWindowReady,
  takePendingMainWindowDestination,
} from "@/lib/api/trayUsage";
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

    const wasLast = pending.length === 1;

    setPending((current) => {
      const index = current.indexOf(destination);
      if (index < 0) return current;
      return [...current.slice(0, index), ...current.slice(index + 1)];
    });

    // A tray-initiated reveal holds the window back until this lands, so the
    // user sees the destination rather than watching it arrive. Two frames:
    // the first callback still runs before the browser paints this state
    // change, the second runs once it is on screen.
    //
    // Deliberately not cancelled on cleanup — removing the destination above
    // re-runs this effect immediately, and a cleanup would cancel the frames
    // before they ever fire, stranding the window for the full timeout.
    if (!wasLast) return;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        void acknowledgeMainWindowReady().catch(() => undefined);
      });
    });
  }, [openProviderSettings, openUsage, pending]);
}
