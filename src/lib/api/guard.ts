import { invoke } from "@tauri-apps/api/core";
import type { AllowanceUsd, GuardAllowanceResult } from "@/types";
import type {
  ConvertedBudget,
  GuardChangedResult,
  UpgradeNotice,
} from "@/types/upgradeNotice";
import { debugUpgradeNotice } from "@/lib/upgradeNoticeDebug";

/**
 * PRO-2 §8 commands. The backend may not be registered yet: a rejection or a
 * payload that is not an `UpgradeNotice` is a missing notice, not a crash.
 */
export async function guardGetUpgradeNotice(): Promise<UpgradeNotice | null> {
  const value = await invoke<unknown>("guard_get_upgrade_notice");
  if (value == null) return null;
  const notice = parseUpgradeNotice(value);
  if (notice == null) {
    const error = new Error(
      "guard_get_upgrade_notice returned an unexpected payload",
    );
    debugUpgradeNotice(error);
    return null;
  }
  return notice;
}

export function guardAckUpgradeNotice(
  noticeId: string,
): Promise<GuardChangedResult> {
  return invoke<GuardChangedResult>("guard_ack_upgrade_notice", {
    noticeId,
  }).then(readChanged);
}

export function guardSetBudgetHardStop(input: {
  budgetId: string;
  hardStop: boolean;
}): Promise<GuardChangedResult> {
  return invoke<GuardChangedResult>("guard_set_budget_hard_stop", {
    budgetId: input.budgetId,
    hardStop: input.hardStop,
  }).then(readChanged);
}

/**
 * Same command as the hold panel. Omit `amountUsd` so the backend applies
 * `allowMoreTodayDefaultUsd`.
 */
export function guardAllowMoreToday(
  amountUsd?: AllowanceUsd,
): Promise<GuardAllowanceResult> {
  if (amountUsd == null) {
    return invoke<GuardAllowanceResult>("guard_allow_more_today");
  }
  return invoke<GuardAllowanceResult>("guard_allow_more_today", {
    amountUsd,
  });
}

const ALLOWANCE_USD = new Set<string>(["2", "5", "10", "20"]);

export function readAllowanceUsd(value: unknown): AllowanceUsd {
  return typeof value === "string" && ALLOWANCE_USD.has(value)
    ? (value as AllowanceUsd)
    : "5";
}

export async function readAllowMoreTodayDefaultUsd(): Promise<AllowanceUsd> {
  try {
    const prefs = await invoke<{
      guard?: { allowMoreTodayDefaultUsd?: unknown };
    }>("get_ui_prefs");
    return readAllowanceUsd(prefs?.guard?.allowMoreTodayDefaultUsd);
  } catch (error) {
    debugUpgradeNotice(error);
    return "5";
  }
}

function parseUpgradeNotice(value: unknown): UpgradeNotice | null {
  if (value == null || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.noticeId !== "string" || raw.noticeId === "") return null;
  if (typeof raw.convertedAt !== "string") return null;
  if (!Array.isArray(raw.budgets)) return null;
  const budgets: ConvertedBudget[] = [];
  for (const item of raw.budgets) {
    const budget = parseConvertedBudget(item);
    if (budget == null) return null;
    budgets.push(budget);
  }
  return {
    noticeId: raw.noticeId,
    convertedAt: raw.convertedAt,
    budgets,
  };
}

function parseConvertedBudget(value: unknown): ConvertedBudget | null {
  if (value == null || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.budgetId !== "string" || raw.budgetId === "") return null;
  if (typeof raw.name !== "string") return null;
  if (raw.providerId !== null && typeof raw.providerId !== "string") {
    return null;
  }
  if (typeof raw.dailyCapUsd !== "string") return null;
  if (typeof raw.spentTodayUsd !== "string") return null;
  if (typeof raw.overToday !== "boolean") return null;
  return {
    budgetId: raw.budgetId,
    name: raw.name,
    providerId: raw.providerId,
    dailyCapUsd: raw.dailyCapUsd,
    spentTodayUsd: raw.spentTodayUsd,
    overToday: raw.overToday,
  };
}

function readChanged(value: unknown): GuardChangedResult {
  if (
    value != null &&
    typeof value === "object" &&
    "changed" in value &&
    typeof value.changed === "boolean"
  ) {
    return { changed: value.changed };
  }
  throw new Error("guard command returned an unexpected result");
}
