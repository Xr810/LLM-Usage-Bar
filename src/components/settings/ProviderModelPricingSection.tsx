import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  useDeleteProviderModelPricing,
  useModelPricing,
  useProviderModelPricing,
  useUpdateProviderModelPricing,
} from "@/lib/query/usage";
import { isNonNegativeDecimalString } from "@/types/usage";
import { useSystemProviderModels } from "@/lib/query/usageDashboard";
import type { ProviderModelPricingView } from "@/types/usageDashboard";

interface ProviderModelPricingSectionProps {
  providerId: string;
  providerName: string;
  /** Credential version the model lookup authenticates with. */
  credentialVersion: number;
}

interface DraftPrice {
  modelId: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

const EMPTY_DRAFT: DraftPrice = {
  modelId: "",
  inputCostPerMillion: "",
  outputCostPerMillion: "",
  cacheReadCostPerMillion: "",
  cacheCreationCostPerMillion: "",
};

const PRICE_FIELDS = [
  {
    key: "inputCostPerMillion",
    labelKey: "usageDashboard.customPricingInputLabel",
    fallback: "Input (USD / 1M tokens)",
  },
  {
    key: "outputCostPerMillion",
    labelKey: "usageDashboard.customPricingOutputLabel",
    fallback: "Output (USD / 1M tokens)",
  },
  {
    key: "cacheReadCostPerMillion",
    labelKey: "usageDashboard.customPricingCacheReadLabel",
    fallback: "Cache read (USD / 1M tokens)",
  },
  {
    key: "cacheCreationCostPerMillion",
    labelKey: "usageDashboard.customPricingCacheWriteLabel",
    fallback: "Cache write (USD / 1M tokens)",
  },
] as const satisfies ReadonlyArray<{
  key: keyof Omit<DraftPrice, "modelId">;
  labelKey: string;
  fallback: string;
}>;

function draftFrom(row: ProviderModelPricingView): DraftPrice {
  // A null rate was left blank and inherits the official one; the box shows it
  // blank so re-saving does not silently pin today's official rate.
  return {
    modelId: row.modelId,
    inputCostPerMillion: row.inputCostPerMillion ?? "",
    outputCostPerMillion: row.outputCostPerMillion ?? "",
    cacheReadCostPerMillion: row.cacheReadCostPerMillion ?? "",
    cacheCreationCostPerMillion: row.cacheCreationCostPerMillion ?? "",
  };
}

/**
 * Per-account model prices for one metered Provider.
 *
 * A price entered here is what the user actually pays, so it outranks both the
 * upstream-reported cost and the built-in official catalogue. Models left
 * unpriced fall back to those, in that order. Edits apply to usage collected
 * from now on; already-recorded usage keeps the price that applied at the time.
 */
export function ProviderModelPricingSection({
  providerId,
  providerName,
  credentialVersion,
}: ProviderModelPricingSectionProps) {
  const { t } = useTranslation();
  const pricingQuery = useProviderModelPricing(providerId);
  const updatePricing = useUpdateProviderModelPricing();
  const deletePricing = useDeleteProviderModelPricing();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<DraftPrice>(EMPTY_DRAFT);
  const [editingModelId, setEditingModelId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Only while the editor is open: it costs an upstream request, and a Provider
  // without a usable key just fails, which is "no suggestions" rather than an
  // error worth putting on screen.
  const providerModels = useSystemProviderModels(
    providerId,
    credentialVersion,
    open,
  );
  const officialPricing = useModelPricing();
  // Matched on the exact ID the user typed. A family ID such as
  // `claude-sonnet-5` is resolved by prefix at pricing time, not here, so the
  // placeholder stays honest about what it actually found.
  const officialForDraft = useMemo(() => {
    const id = draft.modelId.trim().toLowerCase();
    if (!id) return null;
    return (
      (officialPricing.data ?? []).find(
        (entry) => entry.modelId.toLowerCase() === id,
      ) ?? null
    );
  }, [draft.modelId, officialPricing.data]);
  const officialRate = (key: keyof Omit<DraftPrice, "modelId">) =>
    officialForDraft?.[key] ?? null;
  // A stored rate is nullable now, and a null one is inherited rather than
  // absent — rendering it as an empty amount would read as free.
  const rateText = (value: string | null) =>
    value === null
      ? t("usageDashboard.customPricingInheritedRate", {
          defaultValue: "official",
        })
      : `$${value}`;

  const rows = pricingQuery.data ?? [];
  const pending = updatePricing.isPending || deletePricing.isPending;
  const fieldId = (name: string) => `provider-price-${providerId}-${name}`;

  const resetDraft = () => {
    setDraft(EMPTY_DRAFT);
    setEditingModelId(null);
    setError(null);
  };

  const submit = async () => {
    setError(null);
    const modelId = draft.modelId.trim();
    if (!modelId) {
      setError(
        t("usage.modelIdRequired", { defaultValue: "Model ID is required" }),
      );
      return;
    }
    const amounts = [
      draft.inputCostPerMillion,
      draft.outputCostPerMillion,
      draft.cacheReadCostPerMillion,
      draft.cacheCreationCostPerMillion,
    ];
    // A blank box inherits the official rate, so only a filled one is checked.
    if (
      !amounts.every(
        (value) => !value.trim() || isNonNegativeDecimalString(value),
      )
    ) {
      setError(
        t("usage.invalidPrice", {
          defaultValue: "Prices must be non-negative numbers",
        }),
      );
      return;
    }

    try {
      await updatePricing.mutateAsync({
        providerId,
        modelId,
        displayName: modelId,
        price: {
          inputCostPerMillion: draft.inputCostPerMillion.trim(),
          outputCostPerMillion: draft.outputCostPerMillion.trim(),
          cacheReadCostPerMillion: draft.cacheReadCostPerMillion.trim(),
          cacheCreationCostPerMillion: draft.cacheCreationCostPerMillion.trim(),
        },
      });
      resetDraft();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const remove = async (modelId: string) => {
    setError(null);
    try {
      await deletePricing.mutateAsync({ providerId, modelId });
      if (editingModelId === modelId) resetDraft();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="rounded-md border bg-muted/20"
      data-testid={`provider-pricing-${providerId}`}
    >
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left"
        >
          <span className="min-w-0">
            <span className="block text-sm font-medium">
              {t("usageDashboard.customPricingTitle", {
                name: providerName,
                defaultValue: `${providerName} model prices`,
              })}
            </span>
            <span className="block text-xs text-muted-foreground">
              {rows.length === 0
                ? t("usageDashboard.customPricingEmptyHint", {
                    defaultValue:
                      "No custom prices — costs use the reported or official price.",
                  })
                : t("usageDashboard.customPricingCount", {
                    count: rows.length,
                    defaultValue: `${rows.length} model(s) priced at your own rate`,
                  })}
            </span>
          </span>
          <ChevronDown
            aria-hidden="true"
            className={`size-4 shrink-0 text-muted-foreground transition-transform ${
              open ? "rotate-180" : ""
            }`}
          />
        </button>
      </CollapsibleTrigger>

      <CollapsibleContent className="space-y-3 border-t px-3 py-3">
        <p className="text-xs text-muted-foreground">
          {t("usageDashboard.customPricingDescription", {
            defaultValue:
              "Enter what you actually pay per million tokens. Your price overrides the cost this Provider reports. Changes apply to usage collected from now on; usage already recorded keeps the price that applied then.",
          })}
        </p>

        {pricingQuery.isLoading ? (
          <div className="text-sm text-muted-foreground">
            {t("common.loading", { defaultValue: "Loading" })}
          </div>
        ) : null}

        {rows.length > 0 ? (
          <ul className="space-y-1.5">
            {rows.map((row) => (
              <li
                key={row.modelId}
                className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-background/60 px-2.5 py-2"
              >
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium">
                    {row.modelId}
                  </div>
                  <div className="truncate text-xs text-muted-foreground">
                    {t("usageDashboard.customPricingRowSummary", {
                      input: rateText(row.inputCostPerMillion),
                      output: rateText(row.outputCostPerMillion),
                      cacheRead: rateText(row.cacheReadCostPerMillion),
                      cacheWrite: rateText(row.cacheCreationCostPerMillion),
                      defaultValue: `in ${rateText(row.inputCostPerMillion)} · out ${rateText(row.outputCostPerMillion)} · cache read ${rateText(row.cacheReadCostPerMillion)} · cache write ${rateText(row.cacheCreationCostPerMillion)} per 1M`,
                    })}
                  </div>
                </div>
                <div className="flex gap-1.5">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="h-8 px-2.5 text-xs"
                    disabled={pending}
                    onClick={() => {
                      setDraft(draftFrom(row));
                      setEditingModelId(row.modelId);
                      setError(null);
                    }}
                  >
                    {t("common.edit", { defaultValue: "Edit" })}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="h-8 px-2 text-xs"
                    disabled={pending}
                    aria-label={t("usageDashboard.deleteCustomPrice", {
                      model: row.modelId,
                      defaultValue: `Delete custom price for ${row.modelId}`,
                    })}
                    onClick={() => void remove(row.modelId)}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        ) : null}

        <form
          noValidate
          className="space-y-3 rounded-md border border-dashed p-2.5"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor={fieldId("model")}>
              {t("usageDashboard.customPricingModelId", {
                defaultValue: "Model ID",
              })}
            </Label>
            <Input
              id={fieldId("model")}
              list={`${fieldId("model")}-options`}
              value={draft.modelId}
              disabled={pending || editingModelId !== null}
              placeholder="claude-sonnet-5"
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  modelId: event.target.value,
                }))
              }
            />
            {/* A datalist suggests without constraining. The field has to stay
                free text: relays serve models their own /v1/models omits, and a
                family ID is deliberately not a literal model ID. */}
            <datalist id={`${fieldId("model")}-options`}>
              {(providerModels.data ?? []).map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
            <p className="text-xs text-muted-foreground">
              {t("usageDashboard.customPricingModelIdHint", {
                defaultValue:
                  "A family ID such as claude-sonnet-5 also covers its dated variants.",
              })}
            </p>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            {PRICE_FIELDS.map(({ key, labelKey, fallback }) => (
              <div key={key} className="space-y-1.5">
                <Label htmlFor={fieldId(key)}>
                  {t(labelKey, { defaultValue: fallback })}
                </Label>
                <Input
                  id={fieldId(key)}
                  type="number"
                  min="0"
                  step="0.0001"
                  value={draft[key]}
                  disabled={pending}
                  // The placeholder is the rate this box will actually use if
                  // left empty, so a blank field states its own meaning rather
                  // than looking unset.
                  placeholder={
                    officialRate(key) ??
                    t("usageDashboard.customPricingNoOfficialRate", {
                      defaultValue: "No official rate",
                    })
                  }
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      [key]: event.target.value,
                    }))
                  }
                />
              </div>
            ))}
          </div>

          <div className="flex flex-wrap gap-2">
            <Button type="submit" size="sm" disabled={pending}>
              <Plus className="mr-1.5 size-3.5" />
              {editingModelId
                ? t("usageDashboard.saveCustomPrice", {
                    defaultValue: "Save price",
                  })
                : t("usageDashboard.addCustomPrice", {
                    defaultValue: "Add price",
                  })}
            </Button>
            {editingModelId ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={pending}
                onClick={resetDraft}
              >
                {t("common.cancel", { defaultValue: "Cancel" })}
              </Button>
            ) : null}
          </div>

          {error ? (
            <div role="alert" className="text-sm text-destructive">
              {error}
            </div>
          ) : null}
        </form>
      </CollapsibleContent>
    </Collapsible>
  );
}
