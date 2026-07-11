import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { RouteBinding, UsageProviderView } from "@/types/usageDashboard";
import { useTranslation } from "react-i18next";

const protocols = ["claude", "codex", "gemini"] as const;

interface Props {
  providers: UsageProviderView[];
  bindings: RouteBinding[];
  onSave: (protocol: string, providerId: string) => Promise<unknown>;
}

export function RouteBindingsPanel({ providers, bindings, onSave }: Props) {
  const { t } = useTranslation();
  const eligible = providers.filter(
    (provider) => provider.enabled && provider.billingKind === "metered",
  );
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => {
    setValues(
      Object.fromEntries(
        bindings.map((binding) => [binding.protocol, binding.providerId]),
      ),
    );
  }, [bindings]);

  const save = async () => {
    await Promise.all(
      protocols.flatMap((protocol) =>
        values[protocol] ? [onSave(protocol, values[protocol])] : [],
      ),
    );
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("usageDashboard.routes", { defaultValue: "Static routes" })}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-3">
          {protocols.map((protocol) => (
            <label key={protocol} className="space-y-1 text-sm">
              <span className="capitalize">{protocol} route</span>
              <select
                aria-label={`${protocol[0].toUpperCase()}${protocol.slice(1)} route`}
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
        <Button size="sm" onClick={() => void save()}>
          {t("usageDashboard.saveRoutes", { defaultValue: "Save routes" })}
        </Button>
      </CardContent>
    </Card>
  );
}
