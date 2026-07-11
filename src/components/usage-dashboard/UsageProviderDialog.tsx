import { useEffect, useState } from "react";
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
import { Textarea } from "@/components/ui/textarea";
import type {
  BillingKind,
  TokenSource,
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";
import { useTranslation } from "react-i18next";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  provider?: UsageProviderView | null;
  onSave: (input: UsageProviderInput) => Promise<unknown>;
}

export function UsageProviderDialog({
  open,
  onOpenChange,
  provider,
  onSave,
}: Props) {
  const { t } = useTranslation();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [productGroupId, setProductGroupId] = useState("");
  const [billingKind, setBillingKind] = useState<BillingKind>("metered");
  const [tokenSources, setTokenSources] = useState<TokenSource[]>(["proxy"]);
  const [quotaSource, setQuotaSource] = useState("");
  const [quotaIntervalSeconds, setQuotaIntervalSeconds] = useState("300");
  const [routeAppType, setRouteAppType] = useState("claude");
  const [baseUrl, setBaseUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [quotaConfig, setQuotaConfig] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setId(provider?.id ?? "");
    setName(provider?.name ?? "");
    setProductGroupId(provider?.productGroupId ?? "");
    setBillingKind(provider?.billingKind ?? "metered");
    setTokenSources(provider?.tokenSources ?? ["proxy"]);
    setQuotaSource(provider?.quotaSource ?? "");
    setQuotaIntervalSeconds(String(provider?.quotaIntervalSeconds ?? 300));
    setRouteAppType(provider?.routeAppType ?? "claude");
    setBaseUrl(provider?.routeBaseUrl ?? "");
    setSecret("");
    setQuotaConfig("");
    setEnabled(provider?.enabled ?? true);
    setError(null);
  }, [open, provider]);

  const submit = async () => {
    setError(null);
    let parsedQuotaConfig: Record<string, unknown> | undefined;
    if (quotaConfig.trim()) {
      try {
        parsedQuotaConfig = JSON.parse(quotaConfig) as Record<string, unknown>;
      } catch {
        setError(
          t("usageDashboard.invalidQuotaConfig", {
            defaultValue: "Quota config must be valid JSON",
          }),
        );
        return;
      }
    }
    const routeConfig =
      billingKind === "metered" && secret.trim()
        ? { baseUrl: baseUrl.trim(), apiKey: secret.trim() }
        : !provider && billingKind === "metered" && baseUrl.trim()
          ? { baseUrl: baseUrl.trim() }
          : undefined;
    await onSave({
      id: id.trim(),
      name: name.trim(),
      productGroupId: productGroupId.trim(),
      billingKind,
      tokenSources,
      quotaSource:
        billingKind === "subscription" ? quotaSource.trim() || null : null,
      quotaIntervalSeconds:
        billingKind === "subscription"
          ? Number(quotaIntervalSeconds) || 300
          : null,
      routeAppType: billingKind === "metered" ? routeAppType : null,
      routeConfig,
      quotaConfig: parsedQuotaConfig,
      enabled,
    });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {t("usageDashboard.provider", { defaultValue: "Usage Provider" })}
          </DialogTitle>
          <DialogDescription>
            {t("usageDashboard.providerDescription", {
              defaultValue:
                "Configure billing, token sources and optional routing or quota credentials.",
            })}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3 px-6">
          <Label>
            ID
            <Input
              value={id}
              disabled={Boolean(provider)}
              onChange={(event) => setId(event.target.value)}
            />
          </Label>
          <Label>
            {t("common.name", { defaultValue: "Name" })}
            <Input
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </Label>
          <Label>
            {t("usageDashboard.productGroup", {
              defaultValue: "Product group",
            })}
            <Input
              value={productGroupId}
              onChange={(event) => setProductGroupId(event.target.value)}
            />
          </Label>
          <Label>
            {t("usageDashboard.billing", { defaultValue: "Billing" })}
            <select
              className="mt-1 h-9 w-full rounded-md border border-input bg-background px-3"
              value={billingKind}
              onChange={(event) =>
                setBillingKind(event.target.value as BillingKind)
              }
            >
              <option value="metered">Metered</option>
              <option value="subscription">Subscription</option>
            </select>
          </Label>
          <fieldset className="space-y-1">
            <legend className="text-sm font-medium">
              {t("usageDashboard.tokenSources", {
                defaultValue: "Token sources",
              })}
            </legend>
            <div className="flex gap-4 text-sm">
              {(["proxy", "session_log"] as TokenSource[]).map((source) => (
                <label key={source} className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={tokenSources.includes(source)}
                    onChange={(event) =>
                      setTokenSources((current) =>
                        event.target.checked
                          ? [...new Set([...current, source])]
                          : current.filter((item) => item !== source),
                      )
                    }
                  />
                  {source === "proxy" ? "Proxy" : "Session log"}
                </label>
              ))}
            </div>
          </fieldset>
          {billingKind === "metered" ? (
            <>
              <Label>
                {t("usageDashboard.routeProtocol", {
                  defaultValue: "Route protocol",
                })}
                <select
                  className="mt-1 h-9 w-full rounded-md border border-input bg-background px-3"
                  value={routeAppType}
                  onChange={(event) => setRouteAppType(event.target.value)}
                >
                  <option value="claude">Claude</option>
                  <option value="codex">Codex</option>
                  <option value="gemini">Gemini</option>
                </select>
              </Label>
              <Label>
                Base URL
                <Input
                  value={baseUrl}
                  disabled={Boolean(provider?.hasRouteCredentials && !secret)}
                  onChange={(event) => setBaseUrl(event.target.value)}
                />
              </Label>
              <Label>
                {t("usageDashboard.secret", {
                  defaultValue: "API key (leave blank to keep stored key)",
                })}
                <Input
                  type="password"
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                />
              </Label>
              {provider?.hasRouteCredentials && !secret ? (
                <div className="text-xs text-muted-foreground">
                  {t("usageDashboard.storedSecret", {
                    defaultValue:
                      "A credential is stored. Leave the key blank to keep it; re-enter it before changing Base URL.",
                  })}
                </div>
              ) : null}
            </>
          ) : (
            <>
              <Label>
                {t("usageDashboard.quotaSource", {
                  defaultValue: "Quota source",
                })}
                <Input
                  value={quotaSource}
                  onChange={(event) => setQuotaSource(event.target.value)}
                  placeholder="claude / codex / coding_plan"
                />
              </Label>
              <Label>
                {t("usageDashboard.quotaInterval", {
                  defaultValue: "Refresh interval (seconds)",
                })}
                <Input
                  type="number"
                  min={60}
                  value={quotaIntervalSeconds}
                  onChange={(event) =>
                    setQuotaIntervalSeconds(event.target.value)
                  }
                />
              </Label>
              <Label>
                {t("usageDashboard.quotaConfig", {
                  defaultValue:
                    "Quota credential JSON (leave blank to keep stored config)",
                })}
                <Textarea
                  value={quotaConfig}
                  onChange={(event) => setQuotaConfig(event.target.value)}
                  placeholder='{"apiKey":"..."}'
                />
              </Label>
            </>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => setEnabled(event.target.checked)}
            />
            {t("common.enabled", { defaultValue: "Enabled" })}
          </label>
          {error ? (
            <div role="alert" className="text-sm text-destructive">
              {error}
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <Button onClick={() => void submit()}>
            {t("common.save", { defaultValue: "Save" })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
