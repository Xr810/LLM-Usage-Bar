import { useEffect, useState } from "react";
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
import type { UsageProviderView } from "@/types/usageDashboard";

interface SystemProviderApiKeyDialogProps {
  open: boolean;
  provider: UsageProviderView;
  onOpenChange: (open: boolean) => void;
  onSubmit: (apiKey: string) => Promise<unknown>;
  isPending?: boolean;
}

export function SystemProviderApiKeyDialog({
  open,
  provider,
  onOpenChange,
  onSubmit,
  isPending = false,
}: SystemProviderApiKeyDialogProps) {
  const { t } = useTranslation();
  const [apiKey, setApiKey] = useState("");
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setApiKey("");
    setFailed(false);
  }, [open, provider.id]);

  const changeOpen = (nextOpen: boolean) => {
    setApiKey("");
    setFailed(false);
    onOpenChange(nextOpen);
  };

  const submit = async () => {
    const transientKey = apiKey;
    setApiKey("");
    setFailed(false);
    try {
      await onSubmit(transientKey);
      changeOpen(false);
    } catch {
      // Do not surface upstream errors here: an error message must never echo
      // request headers or the submitted credential.
      setFailed(true);
    } finally {
      setApiKey("");
    }
  };

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {t("usageDashboard.saveSystemProviderApiKey", {
              defaultValue: `Save ${provider.name} API key`,
              name: provider.name,
            })}
          </DialogTitle>
          <DialogDescription>
            {t("usageDashboard.systemProviderApiKeyDescription", {
              defaultValue:
                "The upstream key is sent directly to protected storage and is never displayed again.",
            })}
          </DialogDescription>
        </DialogHeader>
        <Label>
          {t("usageDashboard.apiKey", { defaultValue: "API key" })}
          <Input
            type="password"
            autoComplete="off"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
          />
        </Label>
        {failed ? (
          <div role="alert" className="text-sm text-destructive">
            {t("usageDashboard.apiKeySaveFailed", {
              defaultValue: "Unable to save the API key.",
            })}
          </div>
        ) : null}
        <DialogFooter>
          <Button
            disabled={isPending || apiKey.length === 0}
            onClick={() => void submit()}
          >
            {t("usageDashboard.saveApiKey", {
              defaultValue: "Save API key",
            })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
