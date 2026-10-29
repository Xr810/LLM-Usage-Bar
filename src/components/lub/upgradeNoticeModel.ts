import type { LicenseStatus } from "@/types/license";
import type { ConvertedBudget, UpgradeNotice } from "@/types";

/**
 * Community, Free (including an expired Pro, `state: "free"`), and a missing
 * license or notice never render the card. `pro` and `pro_p3` in active or
 * grace do, once the backend returns a notice.
 */
export function upgradeNoticeVisible(
  notice: UpgradeNotice | null | undefined,
  license: LicenseStatus | null | undefined,
): notice is UpgradeNotice {
  if (notice == null || license == null) return false;
  if (license.build !== "official") return false;
  if (license.tier !== "pro" && license.tier !== "pro_p3") return false;
  if (license.state === "free") return false;
  return true;
}

export function convertedBudgetName(
  budget: ConvertedBudget,
  sharedName: string,
): string {
  // Shared budgets send name: null. A blank string is the same row, kept
  // so an older payload does not render an empty label.
  if (
    budget.budgetId === "shared" ||
    budget.name == null ||
    budget.name.trim() === ""
  ) {
    return sharedName;
  }
  return budget.name;
}

/** Prefix a USD decimal string. Values that already include `$` are kept. */
export function usdLabel(value: string): string {
  const trimmed = value.trim();
  if (trimmed.startsWith("$")) return trimmed;
  return `$${trimmed}`;
}

export function partitionConvertedBudgets(
  budgets: readonly ConvertedBudget[],
  switchedIds: ReadonlySet<string>,
): { over: ConvertedBudget[]; rest: ConvertedBudget[] } {
  const over: ConvertedBudget[] = [];
  const rest: ConvertedBudget[] = [];
  for (const budget of budgets) {
    if (budget.overToday && !switchedIds.has(budget.budgetId)) {
      over.push(budget);
    } else {
      rest.push(budget);
    }
  }
  return { over, rest };
}

/** zh and ja list names with an ideographic comma. Other locales use ", ". */
export function joinBudgetNames(
  names: readonly string[],
  language: string,
): string {
  const base = language.toLowerCase();
  const separator =
    base.startsWith("zh") || base.startsWith("ja") ? "、" : ", ";
  return names.join(separator);
}
