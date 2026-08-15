import type { UsageRangePreset, UsageRangeSelection } from "@/types/usage";

const DAY_SECONDS = 24 * 60 * 60;

export interface ResolvedUsageRange {
  startDate: number;
  endDate: number;
}

function getStartOfLocalDayDate(nowMs: number): Date {
  const date = new Date(nowMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function getPresetLookbackStart(
  preset: Exclude<UsageRangePreset, "today" | "1d" | "custom">,
  nowMs: number,
): number {
  const dayCount =
    preset === "7d" ? 7 : preset === "14d" ? 14 : preset === "30d" ? 30 : 365;
  const start = getStartOfLocalDayDate(nowMs);
  start.setDate(start.getDate() - (dayCount - 1));
  return Math.floor(start.getTime() / 1_000);
}

export function resolveProviderActivityRange(
  nowMs: number = Date.now(),
): ResolvedUsageRange {
  const today = getStartOfLocalDayDate(nowMs);
  const start = new Date(today);
  start.setDate(start.getDate() - 364);
  const end = new Date(today);
  end.setDate(end.getDate() + 1);
  return {
    startDate: Math.floor(start.getTime() / 1_000),
    endDate: Math.floor(end.getTime() / 1_000),
  };
}

export function resolveUsageRange(
  selection: UsageRangeSelection,
  nowMs: number = Date.now(),
): ResolvedUsageRange {
  // Backend ranges are half-open (`startAt <= event < endAt`). Advance the
  // live end to the next second so an event timestamped in the current second
  // is visible immediately.
  const endDate = Math.floor(nowMs / 1000) + 1;

  switch (selection.preset) {
    case "today":
      return {
        startDate: Math.floor(getStartOfLocalDayDate(nowMs).getTime() / 1000),
        endDate,
      };
    case "1d":
      return {
        startDate: endDate - DAY_SECONDS,
        endDate,
      };
    case "7d":
    case "14d":
    case "30d":
    case "1y":
      return {
        startDate: getPresetLookbackStart(selection.preset, nowMs),
        endDate,
      };
    case "custom": {
      const startDate = selection.customStartDate ?? endDate - DAY_SECONDS;
      const customEndDate = selection.liveEndTime
        ? endDate
        : (selection.customEndDate ?? endDate);
      return {
        startDate,
        endDate: customEndDate,
      };
    }
  }
}

export function getUsageRangePresetLabel(
  preset: UsageRangePreset,
  t: (key: string, options?: { defaultValue?: string }) => string,
): string {
  switch (preset) {
    case "today":
      return t("usage.presetToday", { defaultValue: "当天" });
    case "1d":
      return t("usage.preset1d", { defaultValue: "1d" });
    case "7d":
      return t("usage.preset7d", { defaultValue: "7d" });
    case "14d":
      return t("usage.preset14d", { defaultValue: "14d" });
    case "30d":
      return t("usage.preset30d", { defaultValue: "30d" });
    case "1y":
      return t("usage.preset1y", { defaultValue: "1 year" });
    case "custom":
      return t("usage.customRange", { defaultValue: "日历筛选" });
  }
}
