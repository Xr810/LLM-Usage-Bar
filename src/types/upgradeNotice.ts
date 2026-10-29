/**
 * PRO-2 §8.2 upgrade notice. Amounts are USD decimal strings, the same
 * normalized form as `allowedToday.usd` and the `*_budget_usd` settings.
 */
export interface UpgradeNotice {
  noticeId: string;
  /** RFC3339 timestamp of the conversion. */
  convertedAt: string;
  budgets: ConvertedBudget[];
}

/**
 * A daily budget that was converted from a reminder into a hard stop.
 * `budgetId` is `"shared"` or `"provider:<providerId>"`.
 * `name` is empty for the shared budget; the UI localizes that row.
 */
export interface ConvertedBudget {
  budgetId: string;
  name: string;
  providerId: string | null;
  dailyCapUsd: string;
  spentTodayUsd: string;
  overToday: boolean;
}

export interface GuardChangedResult {
  changed: boolean;
}
