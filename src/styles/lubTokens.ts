// LLM Usage Bar v5 design tokens (baseline 2026-10-25, frozen). Mirror of tokens.css.
// Target path (assumption): src/styles/lubTokens.ts. Use for JS-side values (recharts colors, layout math).
// Colors here are for canvas/SVG charts only; DOM styling must use the CSS variables.
export type LubTheme = "light" | "dark";
export type LubPlatform = "mac" | "windows";

export const lubColor = {
  light: {
    canvas: "#ecedf0",
    panel: "rgba(246,246,248,0.88)",
    group: "#ffffff",
    ink: "#1d1d1f",
    ink2: "#6e6e73",
    hairline: "rgba(60,60,67,0.13)",
    track: "rgba(120,120,128,0.14)",
    accent: "#0071e3",
    onAccent: "#ffffff",
    ok: "#248a3d",
    /** Status only. Not a meter fill, switch, Beta tag, button, or cap line. */
    warn: "#b86e00",
    stop: "#d70015",
    capDay: "#1d1d1f",
    capMonth: "rgba(29,29,31,0.35)",
    tooltipBg: "#1d1d1f",
  },
  dark: {
    canvas: "#141518",
    panel: "rgba(38,38,41,0.88)",
    group: "rgba(255,255,255,0.06)",
    ink: "#f5f5f7",
    ink2: "#a1a1a6",
    hairline: "rgba(255,255,255,0.10)",
    track: "rgba(255,255,255,0.12)",
    accent: "#0a84ff",
    onAccent: "#ffffff",
    ok: "#30d158",
    /** Status only. Not a meter fill, switch, Beta tag, button, or cap line. */
    warn: "#ffb340",
    stop: "#ff6961",
    capDay: "#f5f5f7",
    capMonth: "rgba(245,245,247,0.42)",
    tooltipBg: "#48484a",
  },
} as const;

export const lubPlatformOverride = {
  windows: {
    light: { accent: "#005fb8", panel: "rgba(243,243,243,0.94)" },
    dark: {
      accent: "#60cdff",
      onAccent: "#00263d",
      panel: "rgba(36,36,36,0.95)",
    },
  },
  mac: { light: {}, dark: {} },
} as const;

export const lubRadius = { mac: [14, 10], windows: [8, 6] } as const;

export const lubType = {
  hero: { size: 40, lineHeight: 44, weight: 700, letterSpacing: "-0.02em" },
  title: { size: 17, lineHeight: 22, weight: 600 },
  body: { size: 13, lineHeight: 18, weight: 400 },
  sub: { size: 12, lineHeight: 16, weight: 400 },
  caption: { size: 11, lineHeight: 14, weight: 400 },
} as const;

export const lubSpace = [0, 4, 8, 12, 16, 20, 24] as const;

export const lubBar = {
  height: 8,
  thinHeight: 4,
  capWidth: 2,
  capHitArea: 28,
  /** bar max = cap × scale → cap line at 1/scale = 83.3% */
  scale: 1.2,
} as const;

/** Trends: single hue, series opacities by rank (Anthropic, OpenAI, OpenRouter...). */
export const lubSeriesOpacity = [1, 0.55, 0.28] as const;
/** Heatmap: accent at 5 steps (0 = track). */
export const lubHeatSteps = [0, 0.25, 0.5, 0.75, 1] as const;

export function lubColors(theme: LubTheme, platform: LubPlatform) {
  return { ...lubColor[theme], ...lubPlatformOverride[platform][theme] };
}
