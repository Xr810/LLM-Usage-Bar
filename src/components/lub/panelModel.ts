import type { TrayCostQuality } from "@/types/trayUsage";

/** C18 default until get_ui_prefs is wired. FE-3 reads this from mock prefs. */
export const DEFAULT_ALERT_THRESHOLD_PCT = 80;

export type LubPanelState = "ok" | "warn" | "capped" | "killed";

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
  /** Percent of the row track. Only set when a beta cut-off line is shown. */
  betaCapPercent?: number | null;
  /** Existing affordance when a metered row has no daily budget yet. */
  onSetBudget?: () => void;
}

export interface LubSessionModel {
  id: string;
  alias: string;
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
  sample: boolean;
  stale: boolean;
  betaOn: boolean;
  betaPercent: number | null;
  alertThresholdPct: number;
  /** Sample clock for the stopped banner. Production has no kill command yet. */
  stoppedAt?: string | null;
  onSetSharedBudget?: () => void;
}
