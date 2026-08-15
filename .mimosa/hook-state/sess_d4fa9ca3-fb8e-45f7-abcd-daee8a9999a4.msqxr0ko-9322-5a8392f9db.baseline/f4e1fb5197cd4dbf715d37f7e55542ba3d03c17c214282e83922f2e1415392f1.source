import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  useOfficialPricingLastRefreshAt,
  useOfficialPricingLastImportedCount,
  useRefreshOfficialPricing,
} from "@/lib/query/usage";

function formatRefreshTime(timestamp: number | null | undefined): string {
  if (timestamp === null || timestamp === undefined) return "—";
  return new Date(timestamp * 1000).toLocaleString();
}

export function OfficialPricingRefreshSection() {
  const { t } = useTranslation();
  const freshness = useOfficialPricingLastRefreshAt();
  const importedCount = useOfficialPricingLastImportedCount();
  const refresh = useRefreshOfficialPricing();
  const error = refresh.error ?? freshness.error ?? importedCount.error;

  const refreshNow = async () => {
    try {
      await refresh.mutateAsync();
    } catch {
      // The mutation exposes the backend error below.
    }
  };

  return (
    <section className="space-y-3 rounded-lg border border-border/60 bg-muted/20 p-4">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">
          {t("usageDashboard.officialPricingTitle", {
            defaultValue: "Official model prices",
          })}
        </h3>
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t("usageDashboard.officialPricingDescription", {
            defaultValue:
              "Machine-maintained vendor list prices from models.dev. Your per-Provider prices still take priority.",
          })}
        </p>
      </div>

      <dl className="grid gap-2 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-xs text-muted-foreground">
            {t("usageDashboard.officialPricingLastRefresh", {
              defaultValue: "Last refresh",
            })}
          </dt>
          <dd>{formatRefreshTime(freshness.data)}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">
            {t("usageDashboard.officialPricingImportedModels", {
              defaultValue: "Models imported in the last run",
            })}
          </dt>
          <dd>{importedCount.data ?? "—"}</dd>
        </div>
      </dl>

      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {t("usageDashboard.officialPricingRefreshFailed", {
            defaultValue: "Unable to refresh official model prices: {{error}}",
            error: error instanceof Error ? error.message : String(error),
          })}
        </p>
      ) : null}

      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={
          refresh.isPending || freshness.isLoading || importedCount.isLoading
        }
        onClick={() => void refreshNow()}
      >
        {refresh.isPending
          ? t("usageDashboard.officialPricingRefreshing", {
              defaultValue: "Refreshing…",
            })
          : t("usageDashboard.officialPricingRefreshNow", {
              defaultValue: "Refresh now",
            })}
      </Button>
    </section>
  );
}
