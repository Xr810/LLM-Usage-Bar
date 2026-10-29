import { useCallback, useEffect, useRef, useState } from "react";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import type { SettingsChangedPayload } from "@/i18n";
import {
  guardAckUpgradeNotice,
  guardAllowMoreToday,
  guardGetUpgradeNotice,
  guardSetBudgetHardStop,
  readAllowMoreTodayDefaultUsd,
  type GuardChangedResult,
} from "@/lib/api/guard";
import type { AllowanceUsd, UpgradeNotice } from "@/types";
import { debugUpgradeNotice } from "@/lib/upgradeNoticeDebug";

export interface UpgradeNoticeController {
  notice: UpgradeNotice | null;
  allowanceUsd: AllowanceUsd;
  ack: (noticeId: string) => Promise<void>;
  setHardStop: (budgetId: string) => Promise<GuardChangedResult>;
  allowMoreToday: () => Promise<void>;
}

/**
 * Pulls `guard_get_upgrade_notice` on startup, on `license-changed`, and on
 * `guard-state-changed`. Ack and hard-stop notify only when `{changed}` is
 * true, and that event is dropped when the guard snapshot is unchanged, so
 * those calls re-fetch the notice themselves. Allow-more does the same.
 * A missing command or any other failure becomes a null notice.
 */
export function useUpgradeNotice(): UpgradeNoticeController {
  const [notice, setNotice] = useState<UpgradeNotice | null>(null);
  const [allowanceUsd, setAllowanceUsd] = useState<AllowanceUsd>("5");
  const requestId = useRef(0);
  const dismissedId = useRef<string | null>(null);

  const refreshNotice = useCallback(async () => {
    const id = ++requestId.current;
    try {
      const next = await guardGetUpgradeNotice();
      if (requestId.current !== id) return;
      if (next != null && next.noticeId === dismissedId.current) return;
      setNotice(next);
    } catch (error) {
      debugUpgradeNotice(error);
      if (requestId.current !== id) return;
      setNotice(null);
    }
  }, []);

  const refreshAllowance = useCallback(async () => {
    setAllowanceUsd(await readAllowMoreTodayDefaultUsd());
  }, []);

  useEffect(() => {
    void refreshNotice();
    void refreshAllowance();
  }, [refreshAllowance, refreshNotice]);

  useTauriEvent("license-changed", () => {
    void refreshNotice();
  });

  useTauriEvent("guard-state-changed", () => {
    void refreshNotice();
  });

  useTauriEvent<SettingsChangedPayload>("settings-changed", (payload) => {
    if (payload?.keys == null || payload.keys.includes("guard")) {
      void refreshAllowance();
    }
  });

  const ack = useCallback(
    async (noticeId: string) => {
      dismissedId.current = noticeId;
      setNotice((current) => (current?.noticeId === noticeId ? null : current));
      try {
        const result = await guardAckUpgradeNotice(noticeId);
        if (result.changed) await refreshNotice();
      } catch (error) {
        debugUpgradeNotice(error);
        if (dismissedId.current === noticeId) dismissedId.current = null;
        await refreshNotice();
      }
    },
    [refreshNotice],
  );

  const setHardStop = useCallback(
    async (budgetId: string) => {
      const result = await guardSetBudgetHardStop({
        budgetId,
        hardStop: false,
      });
      if (result.changed) await refreshNotice();
      return result;
    },
    [refreshNotice],
  );

  const allowMoreToday = useCallback(async () => {
    try {
      await guardAllowMoreToday();
      await refreshNotice();
    } catch (error) {
      debugUpgradeNotice(error);
    }
  }, [refreshNotice]);

  return { notice, allowanceUsd, ack, setHardStop, allowMoreToday };
}
