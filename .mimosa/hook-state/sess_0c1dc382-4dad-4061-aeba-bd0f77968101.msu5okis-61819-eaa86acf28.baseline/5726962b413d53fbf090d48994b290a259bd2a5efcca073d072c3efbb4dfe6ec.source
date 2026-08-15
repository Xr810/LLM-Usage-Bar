import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useSetApiBudgetConfig } from "@/lib/query/trayUsage";
import type { ApiBudgetConfig, ApiBudgetMode } from "@/types/trayUsage";

export function ApiBudgetSettings({
  config,
  isLoading = false,
}: {
  config: ApiBudgetConfig;
  isLoading?: boolean;
}) {
  const { t } = useTranslation();
  const mutation = useSetApiBudgetConfig();
  const [inputValue, setInputValue] = useState(
    config.sharedDailyBudgetUsd ?? "",
  );
  const [error, setError] = useState<string | null>(null);
  const perProvider = config.mode === "per_provider";
  const pending = isLoading || mutation.isPending;

  useEffect(() => {
    setInputValue(config.sharedDailyBudgetUsd ?? "");
    setError(null);
  }, [config.sharedDailyBudgetUsd]);

  const persist = async (
    mode: ApiBudgetMode,
    sharedDailyBudgetUsd: string | null,
  ) => {
    setError(null);
    try {
      await mutation.mutateAsync({ mode, sharedDailyBudgetUsd });
      setInputValue(sharedDailyBudgetUsd ?? "");
    } catch {
      setError(
        t("usageDashboard.apiBudgetUpdateFailed", {
          defaultValue: "Unable to update the API budget.",
        }),
      );
    }
  };

  const saveSharedBudget = async () => {
    const trimmed = inputValue.trim();
    const amount = Number(trimmed);
    if (trimmed === "" || !Number.isFinite(amount) || amount <= 0) {
      setError(
        t("usageDashboard.invalidDailyBudget", {
          defaultValue: "Enter a positive amount.",
        }),
      );
      return;
    }
    await persist("shared", trimmed);
  };

  return (
    <div className="space-y-4 rounded-lg border border-border/60 bg-muted/20 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <Label htmlFor="independent-provider-budgets">
            {t("usageDashboard.independentProviderBudgets", {
              defaultValue: "Set a separate limit for each Provider",
            })}
          </Label>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("usageDashboard.independentProviderBudgetsDescription", {
              defaultValue:
                "Off by default. Turn it on to configure different daily limits on individual API Providers.",
            })}
          </p>
        </div>
        <Switch
          id="independent-provider-budgets"
          checked={perProvider}
          disabled={pending}
          onCheckedChange={(checked) =>
            void persist(
              checked ? "per_provider" : "shared",
              config.sharedDailyBudgetUsd,
            )
          }
        />
      </div>

      {!perProvider ? (
        <form
          noValidate
          className="space-y-3 border-t border-border/60 pt-4"
          onSubmit={(event) => {
            event.preventDefault();
            void saveSharedBudget();
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="shared-api-budget-input">
              {t("usageDashboard.sharedApiDailyBudget", {
                defaultValue: "Combined API daily limit (USD)",
              })}
            </Label>
            <p className="text-xs leading-relaxed text-muted-foreground">
              {t("usageDashboard.sharedApiDailyBudgetDescription", {
                defaultValue:
                  "The spending of all enabled API Providers is added together and compared with this single limit.",
              })}
            </p>
            <div className="flex items-center gap-2">
              <Input
                id="shared-api-budget-input"
                type="number"
                min="0.000001"
                step="0.01"
                value={inputValue}
                disabled={pending}
                onChange={(event) => setInputValue(event.target.value)}
              />
              <span className="text-sm text-muted-foreground">USD</span>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" size="sm" disabled={pending}>
              {t("usageDashboard.saveDailyBudget", {
                defaultValue: "Save budget",
              })}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={pending}
              onClick={() => void persist("shared", null)}
            >
              {t("usageDashboard.clearDailyBudget", {
                defaultValue: "Clear budget",
              })}
            </Button>
          </div>
        </form>
      ) : (
        <p className="border-t border-border/60 pt-4 text-xs text-muted-foreground">
          {t("usageDashboard.perProviderBudgetsActiveDescription", {
            defaultValue:
              "Independent limits are active. Configure each enabled API Provider below.",
          })}
        </p>
      )}

      {error ? (
        <div role="alert" className="text-sm text-destructive">
          {error}
        </div>
      ) : null}
    </div>
  );
}
