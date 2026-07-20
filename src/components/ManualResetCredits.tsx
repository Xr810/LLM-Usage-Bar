import { ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { ManualResetCreditView } from "@/types/quota";

export function ManualResetCredits({
  availableCount,
  credits,
  compact = false,
  className,
}: {
  availableCount: number | null;
  credits: ManualResetCreditView[];
  compact?: boolean;
  className?: string;
}) {
  const { t, i18n } = useTranslation();
  if (availableCount == null && credits.length === 0) return null;

  const count = Math.max(0, availableCount ?? credits.length);
  const undisclosedCount = Math.max(0, count - credits.length);
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const dateFormatter = new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
  });
  const hasDetails = credits.length > 0;
  const headerClassName = cn(
    "flex w-full items-center justify-between gap-3 text-left",
    compact ? "px-2.5 py-2" : "px-3 py-2.5",
  );
  const headerContent = (
    <>
      <span
        className={cn(
          "font-medium text-foreground",
          compact ? "text-[11px] leading-4" : "text-xs leading-5",
        )}
      >
        {t("manualResetCredits.summary", {
          defaultValue: "Usage limit resets",
        })}
      </span>
      <span className="flex shrink-0 items-center gap-2">
        <Badge variant="success" className={compact ? "px-1.5" : undefined}>
          {t("manualResetCredits.available", {
            count,
            defaultValue: "{{count}} available",
          })}
        </Badge>
        {hasDetails ? (
          <ChevronDown
            className="h-3.5 w-3.5 text-muted-foreground transition-transform group-data-[state=open]/manual-reset:rotate-180"
            aria-hidden="true"
          />
        ) : null}
      </span>
    </>
  );

  return (
    <Collapsible
      className={cn(
        "group/manual-reset overflow-hidden rounded-lg border border-border/60 bg-muted/15",
        className,
      )}
    >
      {hasDetails ? (
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className={cn(
              headerClassName,
              "transition-colors hover:bg-muted/30 focus-visible:bg-muted/40 focus-visible:!outline-none",
            )}
            aria-label={t("manualResetCredits.toggle", {
              count,
              defaultValue: "Toggle {{count}} usage limit resets",
            })}
          >
            {headerContent}
          </button>
        </CollapsibleTrigger>
      ) : (
        <div className={headerClassName}>{headerContent}</div>
      )}
      {hasDetails ? (
        <CollapsibleContent>
          <ul className="divide-y divide-border/60 border-t border-border/60">
            {credits.map((credit) => {
              const timestamp = Date.parse(credit.expiresAt);
              const expiryText = Number.isFinite(timestamp)
                ? dateFormatter.format(timestamp)
                : credit.expiresAt;
              return (
                <li
                  key={credit.id}
                  className={cn(compact ? "px-2.5 py-2" : "px-3 py-2.5")}
                >
                  <p
                    className={cn(
                      "truncate font-medium text-foreground",
                      compact ? "text-[11px] leading-4" : "text-xs leading-5",
                    )}
                  >
                    {credit.title ||
                      t("manualResetCredits.defaultTitle", {
                        defaultValue: "Full reset",
                      })}
                  </p>
                  <p
                    className={cn(
                      "truncate text-muted-foreground",
                      compact ? "text-[10px] leading-4" : "text-xs leading-5",
                    )}
                    title={expiryText}
                  >
                    {t("manualResetCredits.expiresAt", {
                      value: expiryText,
                      defaultValue: "Expires {{value}}",
                    })}
                  </p>
                </li>
              );
            })}
          </ul>
          {undisclosedCount > 0 ? (
            <p
              className={cn(
                "border-t border-border/60 px-3 py-2 text-muted-foreground",
                compact ? "text-[10px] leading-4" : "text-xs leading-5",
              )}
            >
              {t("manualResetCredits.undisclosed", {
                count: undisclosedCount,
                defaultValue:
                  "{{count}} more reset expiry times were not provided",
              })}
            </p>
          ) : null}
        </CollapsibleContent>
      ) : null}
    </Collapsible>
  );
}
