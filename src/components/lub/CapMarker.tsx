import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { CAP_LINE_RATIO } from "./capMath";

export type CapMarkerKind = "day" | "month" | "beta" | "hit";

interface CapMarkerProps {
  kind: CapMarkerKind;
  label: string;
  tooltip: string;
  /** Fraction of the track. Day and month lines use the shared 83.3% ratio. */
  leftRatio?: number;
  defaultOpen?: boolean;
}

export function CapMarker({
  kind,
  label,
  tooltip,
  leftRatio = CAP_LINE_RATIO,
  defaultOpen = false,
}: CapMarkerProps) {
  const month = kind === "month";
  // Stopped, beta, and the daily cap share one line. Only the month bar
  // changes color, and that color is --lub-cap-month. The line is never red.
  return (
    <Popover defaultOpen={defaultOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={month ? "lub-cap lub-cap-month" : "lub-cap"}
          data-cap-ink={month ? "month" : "day"}
          style={{ left: `${leftRatio * 100}%` }}
          aria-label={label}
          data-lub-marker={kind}
        />
      </PopoverTrigger>
      <PopoverContent
        className="lub-tip"
        side={month ? "bottom" : "top"}
        align="center"
        sideOffset={8}
      >
        {tooltip}
      </PopoverContent>
    </Popover>
  );
}
