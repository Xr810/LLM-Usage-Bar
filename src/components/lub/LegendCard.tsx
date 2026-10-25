import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CapBar } from "./CapBar";
import { CapMarker, type CapMarkerKind } from "./CapMarker";

const NAV = [
  "trends_heatmaps",
  "caps_stopping",
  "providers",
  "alerts",
  "license",
  "failure_policy",
  "clients_export",
  "about",
] as const;

function RichText({ value }: { value: string }) {
  const nodes: ReactNode[] = [];
  const pattern = /<b>|<\/b>|\|/g;
  let bold = false;
  let last = 0;
  let index = 0;
  for (const match of value.matchAll(pattern)) {
    const text = value.slice(last, match.index);
    if (text) {
      nodes.push(bold ? <b key={index}>{text}</b> : text);
      index += 1;
    }
    if (match[0] === "<b>") bold = true;
    else if (match[0] === "</b>") bold = false;
    else {
      nodes.push(<span key={index} className="lub-vl" aria-hidden="true" />);
      index += 1;
    }
    last = (match.index ?? 0) + match[0].length;
  }
  const rest = value.slice(last);
  if (rest) nodes.push(bold ? <b key={index}>{rest}</b> : rest);
  return <>{nodes}</>;
}

function SampleBar({
  thin = false,
  marker,
  at = 0.8,
}: {
  thin?: boolean;
  marker: CapMarkerKind | null;
  at?: number;
}) {
  return (
    <span className="lub-legend-bar" aria-hidden="true">
      <CapBar
        value={thin ? 35 : 55}
        cap={100}
        scale="raw"
        variant={thin ? "thin" : "row"}
        tone="accent"
        label=""
        valueText=""
        marker={
          marker ? (
            <CapMarker kind={marker} leftRatio={at} label="" tooltip="" />
          ) : null
        }
      />
    </span>
  );
}

export function LegendCard() {
  const { t } = useTranslation();
  const rows: Array<{
    thin?: boolean;
    marker: CapMarkerKind | null;
    at?: number;
    key: string;
  }> = [
    { marker: "day", key: "lub.settings.dark_line_spending_stops_here" },
    {
      thin: true,
      marker: "month",
      key: "lub.settings.light_line_monthly_cap_drawn",
    },
    { marker: null, key: "lub.settings.rows_have_no_line_api" },
    {
      marker: "beta",
      at: 0.9,
      key: "lub.settings.plan_cutoff_line_legend",
    },
  ];
  return (
    <div className="lub-card lub-pad" data-testid="legend-card">
      <p className="lub-legend-title">
        <RichText value={t("lub.settings.what_does_the_line_on")} />
      </p>
      {rows.map((row) => (
        <div key={row.key} className="lub-legend-row" data-legend-row="">
          <SampleBar thin={row.thin} marker={row.marker} at={row.at} />
          <p className="lub-legend-copy">
            <RichText
              value={t(row.key, {
                amount: "$3.80",
                amount2: "$5",
              })}
            />
          </p>
        </div>
      ))}
      <p className="lub-legend-copy">
        <RichText value={t("lub.settings.legend_cap_enforced")} />
      </p>
      <p className="lub-legend-copy">
        <RichText value={t("lub.settings.legend_monitor_only")} />
      </p>
    </div>
  );
}

export function CapsLegendWindow() {
  const { t } = useTranslation();
  return (
    <div className="lub-win" data-testid="caps-legend">
      <nav className="lub-side" aria-label={t("lub.settings.caps_stopping")}>
        {NAV.map((key) => (
          <button
            key={key}
            type="button"
            aria-current={key === "caps_stopping" ? "page" : undefined}
            onClick={(event) => event.preventDefault()}
          >
            {key === "license" ? "License" : t(`lub.settings.${key}`)}
          </button>
        ))}
      </nav>
      <div className="lub-main">
        <h1 className="lub-title">{t("lub.settings.caps_stopping")}</h1>
        <LegendCard />
      </div>
    </div>
  );
}
