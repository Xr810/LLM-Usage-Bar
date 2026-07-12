import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { RouteBinding, UsageProviderView } from "@/types/usageDashboard";
import { useTranslation } from "react-i18next";

const protocols = ["claude", "codex", "gemini"] as const;

interface Props {
  providers: UsageProviderView[];
  bindings: RouteBinding[];
  onSave: (protocol: string, providerId: string) => Promise<unknown>;
  isPending?: boolean;
  title?: string;
  description?: string;
}

export function RouteBindingsPanel({
  providers,
  bindings,
  onSave,
  isPending = false,
  title,
  description,
}: Props) {
  const { t } = useTranslation();
  const eligible = providers.filter(
    (provider) => provider.enabled && provider.billingKind === "metered",
  );
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setValues(
      Object.fromEntries(
        bindings.map((binding) => [binding.protocol, binding.providerId]),
      ),
    );
  }, [bindings]);

  const save = async () => {
    setError(null);
    try {
      await Promise.all(
        protocols.flatMap((protocol) =>
          values[protocol] ? [onSave(protocol, values[protocol])] : [],
        ),
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {title ??
            t("usageDashboard.routes", { defaultValue: "Static routes" })}
        </CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-3">
          {protocols.map((protocol) => (
            <label key={protocol} className="space-y-1 text-sm">
              <span className="capitalize">
                {t("usageDashboard.routeLabel", {
                  protocol,
                  defaultValue: `${protocol} route`,
                })}
              </span>
              <select
                aria-label={t("usageDashboard.routeLabel", {
                  protocol: `${protocol[0].toUpperCase()}${protocol.slice(1)}`,
                  defaultValue: `${protocol[0].toUpperCase()}${protocol.slice(1)} route`,
                })}
                disabled={isPending}
                className="h-9 w-full rounded-md border border-input bg-background px-3"
                value={values[protocol] ?? ""}
                onChange={(event) =>
                  setValues((current) => ({
                    ...current,
                    [protocol]: event.target.value,
                  }))
                }
              >
                <option value="">—</option>
                {eligible
                  .filter(
                    (provider) =>
                      !provider.routeAppType ||
                      provider.routeAppType === protocol,
                  )
                  .map((provider) => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name}
                    </option>
                  ))}
              </select>
            </label>
          ))}
        </div>
        {error ? (
          <div role="alert" className="text-sm text-destructive">
            {error}
          </div>
        ) : null}
        <Button size="sm" disabled={isPending} onClick={() => void save()}>
          {t("usageDashboard.saveRoutes", { defaultValue: "Save routes" })}
        </Button>
      </CardContent>
    </Card>
  );
}
