import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { AgentProviderBindingView } from "@/types/usageDashboard";

export type ProtectedBindingKeyMode = "set" | "replace" | "clear";

interface ProtectedBindingKeyDialogProps {
  open: boolean;
  mode: ProtectedBindingKeyMode;
  binding: AgentProviderBindingView;
  onOpenChange: (open: boolean) => void;
  onSetApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ) => Promise<unknown>;
  onReplaceApiKey: (
    bindingId: string,
    expectedVersion: number,
    apiKey: string,
  ) => Promise<unknown>;
  onClearApiKey: (
    bindingId: string,
    expectedVersion: number,
  ) => Promise<unknown>;
  isPending?: boolean;
}

export function ProtectedBindingKeyDialog({
  open,
  mode,
  binding,
  onOpenChange,
  onSetApiKey,
  onReplaceApiKey,
  onClearApiKey,
  isPending = false,
}: ProtectedBindingKeyDialogProps) {
  const { t } = useTranslation();
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const expectedVersion = useRef(binding.credentialVersion);

  useEffect(() => {
    expectedVersion.current = binding.credentialVersion;
    setApiKey("");
    setError(null);
    // Freeze the CAS version for one open operation. A same-binding refetch
    // must not silently adopt a newer protected credential generation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding.id, mode, open]);

  const changeOpen = (nextOpen: boolean) => {
    setApiKey("");
    setError(null);
    onOpenChange(nextOpen);
  };

  const submit = async () => {
    const transientKey = apiKey;
    setApiKey("");
    setError(null);
    try {
      if (mode === "set") {
        await onSetApiKey(binding.id, expectedVersion.current, transientKey);
      } else if (mode === "replace") {
        await onReplaceApiKey(
          binding.id,
          expectedVersion.current,
          transientKey,
        );
      } else {
        await onClearApiKey(binding.id, expectedVersion.current);
      }
      changeOpen(false);
    } catch (cause) {
      setApiKey("");
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const actionLabel =
    mode === "set"
      ? t("dashboardAgents.setApiKey", { defaultValue: "Set API key" })
      : mode === "replace"
        ? t("dashboardAgents.replaceApiKey", {
            defaultValue: "Replace API key",
          })
        : t("dashboardAgents.clearApiKey", {
            defaultValue: "Clear API key",
          });

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{actionLabel}</DialogTitle>
          <DialogDescription>
            {mode === "clear"
              ? t("dashboardAgents.clearKeyDescription", {
                  defaultValue:
                    "Remove the protected binding credential from this device.",
                })
              : t("dashboardAgents.keyDescription", {
                  defaultValue:
                    "The key is sent directly to protected storage and is never displayed again.",
                })}
          </DialogDescription>
        </DialogHeader>
        {mode !== "clear" ? (
          <Label>
            {t("dashboardAgents.apiKey", { defaultValue: "API key" })}
            <Input
              type="password"
              autoComplete="off"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
            />
          </Label>
        ) : null}
        {error ? (
          <div role="alert" className="text-sm text-destructive">
            {error}
          </div>
        ) : null}
        <DialogFooter>
          <Button
            disabled={isPending || (mode !== "clear" && !apiKey)}
            variant={mode === "clear" ? "destructive" : "default"}
            onClick={() => void submit()}
          >
            {actionLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
