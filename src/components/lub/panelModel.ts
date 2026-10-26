import type { TrayBuildType, TrayCostQuality } from "@/types/trayUsage";

/** C18 default until get_ui_prefs is wired. FE-3 reads this from mock prefs. */
export const DEFAULT_ALERT_THRESHOLD_PCT = 80;

export type LubPanelState = "ok" | "warn" | "over" | "capped" | "killed";

/** Official builds and the community build both only alert when level is over. */
export type LubBuild = "official" | "community";

export type LubSubtitleKey =
  "api_key" | "plan_hour_window" | "plan_codex" | "not_proxied";

export interface LubSourceModel {
  id: string;
  name: string;
  monogram: string;
  subtitleKey: LubSubtitleKey;
  enforceable: boolean;
  amountUsd: string | null;
  usedPercent: number | null;
  scaleCapUsd: string | null;
  costQuality: TrayCostQuality;
  /**
   * Subscription row with no usable window. Never painted as 0%.
   * `stale` is 「数据过期」; `error` is 「读取失败」.
   */
  quotaGap?: "stale" | "error" | null;
  /** Percent of the row track. Only set when a beta cut-off line is shown. */
  betaCapPercent?: number | null;
  /** Existing affordance when a metered row has no daily budget yet. */
  onSetBudget?: () => void;
  /**
   * BE-21b. Codes such as `codex_websocket`. Null or absent shows no note.
   */
  noteCodes?: string[] | null;
  /**
   * BE-21b. Reset instant for this row. Null or absent shows no caption.
   */
  nextResetAt?: string | null;
}

export type LubSessionGroup = "session" | "proxy_traffic" | "unattributed";

export interface LubSessionModel {
  id: string;
  alias: string;
  /** C14. Omitted on the design fixture, which is a plain session row. */
  group?: LubSessionGroup;
  /** Sample fixture uses an i18n key. Real client names are data, not copy. */
  clientKey: "client_acme" | "no_client" | null;
  clientLabel: string | null;
  spentUsd: string;
  capUsd: string;
}

export interface LubPanelModel {
  state: LubPanelState;
  todayUsd: string | null;
  todayQuality: TrayCostQuality;
  dailyCapUsd: string | null;
  showDayLine: boolean;
  monthUsd: string | null;
  monthQuality: TrayCostQuality;
  monthlyCapUsd: string | null;
  showMonthLine: boolean;
  spark: number[] | null;
  sources: LubSourceModel[];
  sessions: LubSessionModel[] | null;
  /** C14. Rows whose amount could not be parsed. 0 hides the hint. */
  invalidCostRows?: number;
  /** BE-21b. Null when the snapshot omits `buildType`. */
  buildType?: TrayBuildType | null;
  sample: boolean;
  stale: boolean;
  betaOn: boolean;
  betaPercent: number | null;
  alertThresholdPct: number;
  /** Sample clock for the stopped banner. Production has no kill command yet. */
  stoppedAt?: string | null;
  /** Gray tray bar. The lift control stays enabled and does not call Tauri. */
  licenseLapsed?: boolean;
  onSetSharedBudget?: () => void;
}
