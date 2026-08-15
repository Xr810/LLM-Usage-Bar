import { forwardRef, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSetProviderDailyBudget } from "@/lib/query/trayUsage";

interface ProviderDailyBudgetFieldProps {
  providerId: string;
  providerName: string;
  value: string | null;
  onSave?: (dailyBudgetUsd: string | null) => Promise<unknown>;
  isPending?: boolean;
  targeted?: boolean;
  onTargetHandled?: () => void;
}

export const ProviderDailyBudgetField = forwardRef<
  HTMLInputElement,
  ProviderDailyBudgetFieldProps
>(function ProviderDailyBudgetField(
  {
    providerId,
    providerName,
    value,
    onSave,
    isPending = false,
    targeted = false,
    onTargetHandled,
  },
  forwardedRef,
) {
  const { t } = useTranslation();
  const mutation = useSetProviderDailyBudget();
  const wrapperRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const targetHandledRef = useRef(false);
  const [inputValue, setInputValue] = useState(value ?? "");
  const [localPending, setLocalPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = isPending || localPending || mutation.isPending;
  const inputId = `provider-budget-input-${providerId}`;

  const setInputRef = useCallback(
    (node: HTMLInputElement | null) => {
      inputRef.current = node;
      if (typeof forwardedRef === "function") {
        forwardedRef(node);
      } else if (forwardedRef) {
        forwardedRef.current = node;
      }
    },
    [forwardedRef],
  );

  useEffect(() => {
    setInputValue(value ?? "");
    setError(null);
  }, [providerId, value]);

  useEffect(() => {
    if (!targeted) {
      targetHandledRef.current = false;
      return;
    }
    if (targetHandledRef.current) return;

    const frame = window.requestAnimationFrame(() => {
      wrapperRef.current?.scrollIntoView({ block: "center" });
      inputRef.current?.focus();
      targetHandledRef.current = true;
      onTargetHandled?.();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [onTargetHandled, targeted]);

  const persist = async (dailyBudgetUsd: string | null) => {
    setError(null);
    setLocalPending(true);
    try {
      if (onSave) {
        await onSave(dailyBudgetUsd);
      } else {
        await mutation.mutateAsync({ providerId, dailyBudgetUsd });
      }
      setInputValue(dailyBudgetUsd ?? "");
    } catch {
      setError(
        t("usageDashboard.dailyBudgetUpdateFailed", {
          defaultValue: "Unable to update this Provider budget.",
        }),
      );
    } finally {
      setLocalPending(false);
    }
  };

  const submit = async () => {
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
    await persist(trimmed);
  };

  return (
    <div
      ref={wrapperRef}
      id={`provider-budget-${providerId}`}
      className="rounded-md border bg-muted/20 p-3"
    >
      <form
        noValidate
        className="space-y-3"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <div className="space-y-1.5">
          <Label htmlFor={inputId}>
            {t("usageDashboard.dailyBudgetLabel", {
              name: providerName,
              defaultValue: `${providerName} daily budget (USD)`,
            })}
          </Label>
          <div className="flex items-center gap-2">
            <Input
              ref={setInputRef}
              id={inputId}
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
            onClick={() => void persist(null)}
          >
            {t("usageDashboard.clearDailyBudget", {
              defaultValue: "Clear budget",
            })}
          </Button>
        </div>
        {error ? (
          <div role="alert" className="text-sm text-destructive">
            {error}
          </div>
        ) : null}
      </form>
    </div>
  );
});
