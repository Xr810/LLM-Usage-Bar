import { ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { QuotaResetDetailView } from "@/types/quota";

function windowLabel(
  windowSeconds: number,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (windowSeconds === 18_000) {
    return t("quotaResetDetails.fiveHour", {
      defaultValue: "5-hour allowance",
    });
  }
  if (windowSeconds === 604_800) {
    return t("quotaResetDetails.weekly", {
      defaultValue: "Weekly allowance",
    });
  }
  if (windowSeconds % 86_400 === 0) {
    return t("quotaResetDetails.days", {
      count: windowSeconds / 86_400,
      defaultValue: "{{count}}-day allowance",
    });
  }
  return t("quotaResetDetails.hours", {
    count: Math.max(1, Math.round(windowSeconds / 3_600)),
    defaultValue: "{{count}}-hour allowance",
  });
}

export function QuotaResetDetails({
  details,
  compact = false,
  className,
}: {
  details: QuotaResetDetailView[];
  compact?: boolean;
  className?: string;
}) {
  const { t, i18n } = useTranslation();
  if (details.length === 0) return null;

  const locale = i18n.resolvedLanguage ?? i18n.language;
  const dateFormatter = new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  });

  return (
    <Collapsible className={cn("group/reset-details", className)}>
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex w-full items-center justify-between gap-2 text-left text-muted-foreground transition-colors hover:text-foreground",
            compact ? "text-[11px] leading-4" : "text-xs leading-5",
          )}
          aria-label={t("quotaResetDetails.toggle", {
            count: details.length,
            defaultValue: "Toggle {{count}} additional quota reset times",
          })}
        >
          <span>
            {t("quotaResetDetails.summary", {
              count: details.length,
              defaultValue: "Additional quota resets: {{count}}",
            })}
          </span>
          <ChevronDown
            className="h-3.5 w-3.5 shrink-0 transition-transform group-data-[state=open]/reset-details:rotate-180"
            aria-hidden="true"
          />
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ul
          className={cn(
            "mt-2 space-y-1.5 border-l border-border/60 pl-3",
            compact ? "text-[11px] leading-4" : "text-xs leading-5",
          )}
        >
          {details.map((detail) => {
            const timestamp = Date.parse(detail.resetsAt);
            const resetText = Number.isFinite(timestamp)
              ? dateFormatter.format(timestamp)
              : detail.resetsAt;
            return (
              <li key={detail.id} className="min-w-0">
                <p className="truncate font-medium text-foreground">
                  {detail.label} · {windowLabel(detail.windowSeconds, t)}
                </p>
                <p className="truncate text-muted-foreground" title={resetText}>
                  {t("quotaResetDetails.resetsAt", {
                    value: resetText,
                    defaultValue: "Resets {{value}}",
                  })}
                </p>
              </li>
            );
          })}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  );
}
