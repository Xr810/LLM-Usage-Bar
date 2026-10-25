export type CompareCell =
  { kind: "check" } | { kind: "missing" } | { kind: "text"; key: string };

export interface CompareRow {
  id: string;
  labelKey: string;
  free: CompareCell;
  pro: CompareCell;
  /** Neutral “Beta” text. Never a glyph. */
  beta?: boolean;
}

export interface CompareGroup {
  id: string;
  labelKey: string;
  rows: CompareRow[];
}

const check: CompareCell = { kind: "check" };
const missing: CompareCell = { kind: "missing" };
const text = (key: string): CompareCell => ({ kind: "text", key });

/** Rows live in data so a later Free / Pro split does not require a JSX edit. */
export const compareGroups: CompareGroup[] = [
  {
    id: "see",
    labelKey: "group_see",
    rows: [
      { id: "live", labelKey: "row_live", free: check, pro: check },
      {
        id: "trends",
        labelKey: "row_trends",
        free: text("weeks_52"),
        pro: text("weeks_52"),
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
        labelKey: "row_history",
        free: text("weeks_52"),
        pro: text("coming_soon"),
      },
      {
        id: "log",
        labelKey: "row_log",
        free: text("retention_free"),
        pro: text("retention_pro"),
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
        free: text("caps_alert_only"),
        pro: check,
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
        free: missing,
        pro: check,
      },
      {
        id: "plan-cutoff",
        labelKey: "row_plan_cutoff",
        free: missing,
        pro: check,
        beta: true,
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
  {
    id: "p3",
    labelKey: "group_p3",
    rows: [
      {
        id: "p3",
        labelKey: "row_p3",
        free: missing,
        pro: text("coming_soon"),
      },
    ],
  },
];
