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
import type {
  BillingKind,
  SessionSource,
  TokenSource,
  UsageProviderInput,
  UsageProviderView,
} from "@/types/usageDashboard";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  provider?: UsageProviderView | null;
  onSave: (input: UsageProviderInput) => Promise<unknown>;
  isPending?: boolean;
}

export function UsageProviderDialog({
  open,
  onOpenChange,
  provider,
  onSave,
  isPending = false,
}: Props) {
  const { t } = useTranslation();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [productGroupId, setProductGroupId] = useState("");
  const [billingKind, setBillingKind] = useState<BillingKind>("metered");
  const [tokenSources, setTokenSources] = useState<TokenSource[]>(["proxy"]);
  const [sessionSourceBindings, setSessionSourceBindings] = useState<
    SessionSource[]
  >([]);
  const [quotaSource, setQuotaSource] = useState("");
  const [quotaIntervalSeconds, setQuotaIntervalSeconds] = useState("900");
  const [routeAppType, setRouteAppType] = useState("claude");
  const [baseUrl, setBaseUrl] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setId(provider?.id ?? "");
    setName(provider?.name ?? "");
    setProductGroupId(provider?.productGroupId ?? "");
    setBillingKind(provider?.billingKind ?? "metered");
    setTokenSources(provider?.tokenSources ?? ["proxy"]);
    setSessionSourceBindings(provider?.sessionSourceBindings ?? []);
    setQuotaSource(provider?.quotaSource ?? "");
    setQuotaIntervalSeconds(String(provider?.quotaIntervalSeconds ?? 900));
    setRouteAppType(provider?.routeAppType ?? "claude");
    setBaseUrl(provider?.routeBaseUrl ?? "");
    setEnabled(provider?.enabled ?? true);
    setError(null);
  }, [open, provider]);

  const submit = async () => {
    setError(null);
    const parsedInterval = Number(quotaIntervalSeconds);
    if (
      billingKind === "subscription" &&
      (quotaIntervalSeconds.trim() === "" ||
        !Number.isFinite(parsedInterval) ||
        !Number.isInteger(parsedInterval) ||
        parsedInterval < 0 ||
        (parsedInterval > 0 && parsedInterval < 60))
    ) {
      setError(
        t("usageDashboard.invalidQuotaInterval", {
          defaultValue: "Refresh interval must be 0 or at least 60 seconds.",
        }),
      );
      return;
    }

    const routeConfig =
      billingKind === "metered" &&
      baseUrl.trim() &&
      (!provider || !provider.hasRouteCredentials)
        ? { baseUrl: baseUrl.trim() }
        : undefined;
    const input: UsageProviderInput = {
      id: id.trim(),
      name: name.trim(),
      productGroupId: productGroupId.trim(),
      billingKind,
      tokenSources,
      sessionSourceBindings: tokenSources.includes("session_log")
        ? sessionSourceBindings
        : [],
      quotaSource:
        billingKind === "subscription" ? quotaSource.trim() || null : null,
      quotaIntervalSeconds:
        billingKind === "subscription" ? parsedInterval : null,
      routeAppType: billingKind === "metered" ? routeAppType : null,
      routeConfig,
      enabled,
    };

    try {
      await onSave(input);
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
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
                "Configure Provider billing and data-source metadata. Agent bindings and their keys are managed separately.",
            })}
          </DialogDescription>
        </DialogHeader>
        <div
          data-testid="provider-form-scroll-region"
          className="grid min-h-0 flex-1 gap-3 overflow-y-auto px-6"
        >
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
              <option value="metered">
                {t("usageDashboard.metered", { defaultValue: "Metered" })}
              </option>
              <option value="subscription">
                {t("usageDashboard.subscription", {
                  defaultValue: "Subscription",
                })}
              </option>
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
                  {source === "proxy"
                    ? t("usageDashboard.sourceProxy", { defaultValue: "Proxy" })
                    : t("usageDashboard.sourceSession", {
                        defaultValue: "Session log",
                      })}
                </label>
              ))}
            </div>
          </fieldset>

          <fieldset
            className="space-y-1"
            disabled={!tokenSources.includes("session_log")}
          >
            <legend className="text-sm font-medium">
              {t("usageDashboard.sessionSourceBindings", {
                defaultValue: "Session log ownership",
              })}
            </legend>
            <div className="flex gap-4 text-sm">
              {(["claude", "codex"] as SessionSource[]).map((source) => (
                <label key={source} className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    aria-label={
                      source === "claude"
                        ? t("usageDashboard.claudeSessionSource", {
                            defaultValue: "Claude session logs",
                          })
                        : t("usageDashboard.codexSessionSource", {
                            defaultValue: "Codex session logs",
                          })
                    }
                    checked={sessionSourceBindings.includes(source)}
                    onChange={(event) =>
                      setSessionSourceBindings((current) =>
                        event.target.checked
                          ? [...new Set([...current, source])]
                          : current.filter((item) => item !== source),
                      )
                    }
                  />
                  {source === "claude"
                    ? t("usageDashboard.claudeSessionSource", {
                        defaultValue: "Claude session logs",
                      })
                    : t("usageDashboard.codexSessionSource", {
                        defaultValue: "Codex session logs",
                      })}
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
                  disabled={Boolean(provider?.hasRouteCredentials)}
                  onChange={(event) => setBaseUrl(event.target.value)}
                />
              </Label>
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
                  defaultValue: "Idle refresh interval (seconds)",
                })}
                <Input
                  type="number"
                  min={0}
                  aria-label={t("usageDashboard.quotaInterval", {
                    defaultValue: "Idle refresh interval (seconds)",
                  })}
                  value={quotaIntervalSeconds}
                  onChange={(event) =>
                    setQuotaIntervalSeconds(event.target.value)
                  }
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
            <div
              role="alert"
              aria-label={error}
              className="text-sm text-destructive"
            >
              {error}
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <Button disabled={isPending} onClick={() => void submit()}>
            {t("common.save", { defaultValue: "Save" })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
