export type CompareCell =
  { kind: "check" } | { kind: "missing" } | { kind: "text"; key: string };

export interface CompareRow {
  id: string;
  labelKey: string;
  free: CompareCell;
  pro: CompareCell;
}

export interface CompareGroup {
  id: string;
  labelKey: string;
  rows: CompareRow[];
}

const check: CompareCell = { kind: "check" };
const missing: CompareCell = { kind: "missing" };
const text = (key: string): CompareCell => ({ kind: "text", key });

/**
 * Free vs Pro rows. Keys are the v5.8 `lub.procompare` pack.
 * Price, trial, refund, and P3 price strings are not rows.
 */
export const compareGroups: CompareGroup[] = [
  {
    id: "see",
    labelKey: "group_see",
    rows: [
      { id: "live", labelKey: "row_live", free: check, pro: check },
      {
        id: "trends",
        labelKey: "row_trends",
        free: text("trends_free"),
        pro: text("trends_pro"),
      },
      {
        id: "weekday",
        labelKey: "row_weekday_hour",
        free: missing,
        pro: check,
      },
      {
        id: "split",
        labelKey: "row_split",
        free: missing,
        pro: text("coming_soon"),
      },
      {
        id: "history",
        labelKey: "row_history_24m",
        free: missing,
        pro: text("coming_soon"),
      },
      {
        id: "log",
        labelKey: "row_breaker_log",
        free: text("log_free"),
        pro: text("log_pro"),
      },
    ],
  },
  {
    id: "stop",
    labelKey: "group_stop",
    rows: [
      {
        id: "caps",
        labelKey: "row_caps",
        free: text("caps_free"),
        pro: text("caps_pro"),
      },
      { id: "allow", labelKey: "row_allow", free: missing, pro: check },
      {
        id: "session-kill",
        labelKey: "row_session_kill",
        free: missing,
        pro: check,
      },
      {
        id: "identity",
        labelKey: "row_identity",
        free: text("identity_free"),
        pro: text("identity_pro"),
      },
      {
        id: "plan-cutoff",
        labelKey: "row_plan_cutoff",
        free: missing,
        pro: check,
      },
    ],
  },
  {
    id: "bill",
    labelKey: "group_bill",
    rows: [
      {
        id: "receipts",
        labelKey: "row_receipts",
        free: missing,
        pro: check,
      },
    ],
  },
];

const PRICE_ROW_KEYS = [
  "price_month",
  "price_year",
  "trial",
  "refund",
  "early_bird_founder",
  "beta_free",
  "p3_price",
  "p3_only_with_pro",
  "start_trial",
];

export function compareRowKeys(): string[] {
  return compareGroups.flatMap((group) => [
    group.labelKey,
    ...group.rows.flatMap((row) => {
      const cells = [row.labelKey];
      for (const cell of [row.free, row.pro]) {
        if (cell.kind === "text") cells.push(cell.key);
      }
      return cells;
    }),
  ]);
}

export function compareModelOmitsPrices(): boolean {
  const keys = new Set(compareRowKeys());
  return PRICE_ROW_KEYS.every((key) => !keys.has(key));
}
