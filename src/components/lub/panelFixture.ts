import type { LubPanelModel } from "./panelModel";

/** Same generator as the frozen prototype spark (last 14 of the 30-day series). */
function prototypeRand(index: number): number {
  const value = Math.sin(index * 12.9898) * 43758.5453;
  return value - Math.floor(value);
}

export function prototypeSpark14(): number[] {
  const totals: number[] = [];
  for (let day = 0; day < 30; day += 1) {
    let sum = 0;
    [3.2, 1.6, 0.8].forEach((scale, series) => {
      sum += Number(
        (scale * (0.4 + prototypeRand(day * 3 + series) * 1.3)).toFixed(2),
      );
    });
    totals.push(sum);
  }
  return totals.slice(-14);
}

/** Design-sample tray. Marked sample so the footer shows the prototype badge. */
export function designPanelFixture(
  overrides: Partial<LubPanelModel> = {},
): LubPanelModel {
  return {
    state: "ok",
    todayUsd: "7.42",
    todayQuality: "complete",
    dailyCapUsd: "10",
    showDayLine: true,
    monthUsd: "86.10",
    monthQuality: "complete",
    monthlyCapUsd: "200",
    showMonthLine: true,
    spark: prototypeSpark14(),
    sources: [
      {
        id: "claude-code",
        name: "Claude Code",
        monogram: "A",
        subtitleKey: "api_key",
        enforceable: true,
        amountUsd: "5.10",
        usedPercent: null,
        scaleCapUsd: "10",
        costQuality: "complete",
      },
      {
        id: "codex",
        name: "Codex",
        monogram: "O",
        subtitleKey: "api_key",
        enforceable: true,
        amountUsd: "2.32",
        usedPercent: null,
        scaleCapUsd: "10",
        costQuality: "complete",
      },
      {
        id: "claude-max",
        name: "Claude Max",
        monogram: "A",
        subtitleKey: "plan_hour_window",
        enforceable: false,
        amountUsd: null,
        usedPercent: 62,
        scaleCapUsd: null,
        costQuality: "complete",
      },
      {
        id: "chatgpt-plus",
        name: "ChatGPT Plus",
        monogram: "O",
        subtitleKey: "plan_codex",
        enforceable: false,
        amountUsd: null,
        usedPercent: 41,
        scaleCapUsd: null,
        costQuality: "complete",
      },
      {
        id: "cursor",
        name: "Cursor",
        monogram: "C",
        subtitleKey: "not_proxied",
        enforceable: false,
        amountUsd: "3.00",
        usedPercent: null,
        scaleCapUsd: "10",
        costQuality: "complete",
      },
    ],
    sessions: [
      {
        id: "acme-web",
        alias: "acme-web",
        clientKey: "client_acme",
        clientLabel: null,
        spentUsd: "3.80",
        capUsd: "5",
      },
      {
        id: "quant-workbench",
        alias: "quant-workbench",
        clientKey: "no_client",
        clientLabel: null,
        spentUsd: "1.30",
        capUsd: "5",
      },
    ],
    sample: true,
    stale: false,
    betaOn: false,
    betaPercent: null,
    // The v56 panel reference marks acme-web at $3.80 / $5 (76%) as near.
    // Live sessions keep DEFAULT_ALERT_THRESHOLD_PCT (80).
    alertThresholdPct: 76,
    ...overrides,
  };
}

/** Beta off. The cap line stays the daily ink color; stopped is a label, not a red line. */
export function killedPanelFixture(): LubPanelModel {
  const base = designPanelFixture();
  return {
    ...base,
    state: "killed",
    stoppedAt: "18:41",
    spark: null,
    sessions: null,
    sources: base.sources.filter((source) =>
      ["claude-code", "codex", "claude-max"].includes(source.id),
    ),
  };
}

/** Beta on, still no β mark. Subscription rows carry the shared cap line. */
export function betaPanelFixture(): LubPanelModel {
  const base = designPanelFixture();
  return {
    ...base,
    betaOn: true,
    betaPercent: 90,
    sources: base.sources.map((source) =>
      source.usedPercent == null ? source : { ...source, betaCapPercent: 90 },
    ),
  };
}

export function scrollPanelFixture(): LubPanelModel {
  const base = designPanelFixture();
  const extra = Array.from({ length: 6 }, (_, index) => ({
    id: `extra-${index}`,
    name: `Source ${index + 1}`,
    monogram: String(index + 1),
    subtitleKey: "api_key" as const,
    enforceable: true,
    amountUsd: "1.00",
    usedPercent: null,
    scaleCapUsd: "10",
    costQuality: "complete" as const,
  }));
  return { ...base, sources: [...base.sources, ...extra] };
}
