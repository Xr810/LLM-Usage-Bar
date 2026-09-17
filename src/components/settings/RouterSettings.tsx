import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  routerApi,
  type ModelRoute,
  type RouterProvider,
} from "@/lib/api/router";
import { usageDashboardApi } from "@/lib/api/usageDashboard";

const selectClass =
  "h-9 w-full rounded-md border border-input bg-background px-3 text-sm";

export function RouterSettings({ onManageKeys }: { onManageKeys: () => void }) {
  const { t } = useTranslation();
  const text = (key: string) => t(`router.${key}`);
  const client = useQueryClient();
  const [draft, setDraft] = useState<RouterProvider | null>(null);
  const [routes, setRoutes] = useState<ModelRoute[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [confirm, setConfirm] = useState<
    "connect" | "disconnect" | string | null
  >(null);
  const [days, setDays] = useState(7);
  const providers = useQuery({
    queryKey: ["router", "providers"],
    queryFn: routerApi.list,
  });
  const mode = useQuery({
    queryKey: ["router", "mode"],
    queryFn: routerApi.mode,
  });
  const pointer = useQuery({
    queryKey: ["router", "pointer"],
    queryFn: routerApi.pointer,
  });
  const usage = useQuery({
    queryKey: ["router", "usage", days],
    queryFn: () => routerApi.usage(days),
  });
  const accounts = useQuery({
    queryKey: ["router", "keys"],
    queryFn: usageDashboardApi.listProviders,
  });
  const keys =
    accounts.data?.flatMap((account) =>
      account.apiKeys.map((key) => ({ ...key, accountName: account.name })),
    ) ?? [];

  async function action(task: () => Promise<unknown>, success?: string) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await task();
      await client.invalidateQueries({ queryKey: ["router"] });
      setMessage(success ?? text("saved"));
      setConfirm(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  async function edit(provider: RouterProvider) {
    setBusy(true);
    setError("");
    try {
      const mappings = await routerApi.routes(provider.id);
      setDraft({ ...provider });
      setRoutes(mappings);
      setMessage("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  function add() {
    setDraft({
      id: crypto.randomUUID(),
      displayName: "",
      baseUrl: "",
      wireApi: "responses",
      priority: providers.data?.length ?? 0,
      enabled: true,
      authKind: "bearer_key",
      credentialKeyId: null,
    });
    setRoutes([{ logicalModel: "", upstreamModel: "" }]);
    setError("");
    setMessage("");
  }
  function save() {
    if (!draft) return;
    let url: URL;
    try {
      url = new URL(draft.baseUrl);
    } catch {
      setError(text("invalidUrl"));
      return;
    }
    const mappings = routes.map((route) => ({
      logicalModel: route.logicalModel.trim(),
      upstreamModel: route.upstreamModel.trim(),
    }));
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      setError(text("invalidUrl"));
      return;
    }
    if (
      !draft.displayName.trim() ||
      !Number.isSafeInteger(draft.priority) ||
      mappings.length === 0 ||
      mappings.some((route) => !route.logicalModel || !route.upstreamModel) ||
      new Set(mappings.map((route) => route.logicalModel)).size !==
        mappings.length
    ) {
      setError(text("invalidMappings"));
      return;
    }
    if (draft.authKind === "bearer_key" && !draft.credentialKeyId) {
      setError(text("selectKey"));
      return;
    }
    void action(async () => {
      await routerApi.save({
        ...draft,
        displayName: draft.displayName.trim(),
        baseUrl: draft.baseUrl.trim(),
        credentialKeyId:
          draft.authKind === "bearer_key" ? draft.credentialKeyId : null,
      });
      await routerApi.saveRoutes(draft.id, mappings);
      setDraft(null);
    });
  }
  const loadError =
    providers.error ||
    mode.error ||
    pointer.error ||
    usage.error ||
    accounts.error;
  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{text("description")}</p>
      {(error || loadError) && (
        <div
          role="alert"
          className="rounded-lg border border-destructive p-3 text-sm text-destructive"
        >
          {error || String(loadError)}
          <Button
            variant="ghost"
            onClick={() =>
              void client.invalidateQueries({ queryKey: ["router"] })
            }
          >
            {text("refresh")}
          </Button>
        </div>
      )}
      {message && (
        <p role="status" className="text-sm text-primary">
          {message}
        </p>
      )}
      {providers.isPending ? (
        <p role="status">{text("loading")}</p>
      ) : (
        <>
          <section className="space-y-3 rounded-xl border p-4">
            <div className="flex items-center justify-between">
              <h3 className="font-semibold">{text("providers")}</h3>
              <Button onClick={add} disabled={busy || !!draft}>
                {text("add")}
              </Button>
            </div>
            {!providers.data?.length && (
              <p className="text-sm text-muted-foreground">{text("empty")}</p>
            )}
            {providers.data?.map((provider) => (
              <div
                key={provider.id}
                className="flex items-center justify-between gap-3 rounded-lg bg-muted/40 p-3"
              >
                <div className="min-w-0">
                  <p className="font-medium">
                    {provider.displayName}{" "}
                    <span className="text-xs text-muted-foreground">
                      #{provider.priority} ·{" "}
                      {text(provider.enabled ? "enabled" : "disabled")}
                    </span>
                  </p>
                  <p className="truncate text-xs text-muted-foreground">
                    {provider.baseUrl}
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    disabled={busy || !!draft}
                    onClick={() => void edit(provider)}
                  >
                    {text("edit")}
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={busy || !!draft}
                    onClick={() => setConfirm(`delete:${provider.id}`)}
                  >
                    {text("delete")}
                  </Button>
                </div>
              </div>
            ))}
            {draft && (
              <form
                className="space-y-4 border-t pt-4"
                onSubmit={(event) => {
                  event.preventDefault();
                  save();
                }}
              >
                <fieldset disabled={busy} className="space-y-4">
                  <label className="block space-y-1 text-sm">
                    {text("name")}
                    <Input
                      required
                      value={draft.displayName}
                      onChange={(e) =>
                        setDraft({ ...draft, displayName: e.target.value })
                      }
                    />
                  </label>
                  <label className="block space-y-1 text-sm">
                    {text("url")}
                    <Input
                      required
                      type="url"
                      placeholder="https://api.example.com/v1"
                      value={draft.baseUrl}
                      onChange={(e) =>
                        setDraft({ ...draft, baseUrl: e.target.value })
                      }
                    />
                  </label>
                  <div className="grid grid-cols-2 gap-4">
                    <label className="space-y-1 text-sm">
                      {text("priority")}
                      <Input
                        required
                        type="number"
                        step="1"
                        value={draft.priority}
                        onChange={(e) =>
                          setDraft({
                            ...draft,
                            priority: Number(e.target.value),
                          })
                        }
                      />
                    </label>
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={draft.enabled}
                        onChange={(e) =>
                          setDraft({ ...draft, enabled: e.target.checked })
                        }
                      />
                      {text("enabled")}
                    </label>
                  </div>
                  <label className="block space-y-1 text-sm">
                    {text("auth")}
                    <select
                      className={selectClass}
                      value={draft.authKind}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          authKind: e.target
                            .value as RouterProvider["authKind"],
                        })
                      }
                    >
                      <option value="bearer_key">{text("apiKey")}</option>
                      <option value="chatgpt_oauth">{text("oauth")}</option>
                      <option value="none">{text("noAuth")}</option>
                    </select>
                  </label>
                  {draft.authKind === "bearer_key" && (
                    <div className="space-y-2">
                      <label className="block space-y-1 text-sm">
                        {text("key")}
                        <select
                          className={selectClass}
                          value={draft.credentialKeyId ?? ""}
                          onChange={(e) =>
                            setDraft({
                              ...draft,
                              credentialKeyId: e.target.value || null,
                            })
                          }
                        >
                          <option value="">{text("selectKey")}</option>
                          {keys.map((key) => (
                            <option
                              key={key.id}
                              value={key.id}
                              disabled={key.credentialStatus !== "configured"}
                            >
                              {key.accountName} / {key.label} ·{" "}
                              {key.credentialStatus}
                            </option>
                          ))}
                        </select>
                      </label>
                      <Button
                        type="button"
                        variant="link"
                        onClick={onManageKeys}
                      >
                        {text("manageKeys")}
                      </Button>
                    </div>
                  )}
                  <div className="space-y-2">
                    <h4 className="text-sm font-medium">{text("models")}</h4>
                    <p className="text-xs text-muted-foreground">
                      {text("modelsHelp")}
                    </p>
                    {routes.map((route, index) => (
                      <div className="flex gap-2" key={index}>
                        <Input
                          aria-label={`${text("logical")} ${index + 1}`}
                          required
                          value={route.logicalModel}
                          onChange={(e) =>
                            setRoutes(
                              routes.map((r, i) =>
                                i === index
                                  ? { ...r, logicalModel: e.target.value }
                                  : r,
                              ),
                            )
                          }
                          placeholder={text("logical")}
                        />
                        <Input
                          aria-label={`${text("upstream")} ${index + 1}`}
                          required
                          value={route.upstreamModel}
                          onChange={(e) =>
                            setRoutes(
                              routes.map((r, i) =>
                                i === index
                                  ? { ...r, upstreamModel: e.target.value }
                                  : r,
                              ),
                            )
                          }
                          placeholder={text("upstream")}
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          aria-label={`${text("removeMapping")} ${index + 1}`}
                          onClick={() =>
                            setRoutes(routes.filter((_, i) => i !== index))
                          }
                        >
                          ×
                        </Button>
                      </div>
                    ))}
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() =>
                        setRoutes([
                          ...routes,
                          { logicalModel: "", upstreamModel: "" },
                        ])
                      }
                    >
                      {text("addMapping")}
                    </Button>
                  </div>
                  <div className="flex gap-2">
                    <Button type="submit">{text("save")}</Button>
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => setDraft(null)}
                    >
                      {text("cancel")}
                    </Button>
                  </div>
                </fieldset>
              </form>
            )}
          </section>
          <section className="space-y-3 rounded-xl border p-4">
            <h3 className="font-semibold">{text("connection")}</h3>
            <label className="block space-y-1 text-sm">
              {text("mode")}
              <select
                className={selectClass}
                value={mode.data ?? "auto"}
                disabled={busy || mode.isPending}
                onChange={(e) =>
                  void action(() => routerApi.setMode(e.target.value))
                }
              >
                <option value="auto">{text("auto")}</option>
                {providers.data
                  ?.filter((p) => p.enabled)
                  .map((p) => (
                    <option key={p.id} value={`manual:${p.id}`}>
                      {text("manual")} — {p.displayName}
                    </option>
                  ))}
              </select>
            </label>
            <p className="text-sm">
              {text("pointer")}: {text(pointer.data?.state ?? "loading")}
              {pointer.data?.current && ` (${pointer.data.current})`}
            </p>
            <Button
              disabled={
                busy ||
                pointer.isPending ||
                pointer.data?.state === "unreadable" ||
                (!providers.data?.some((p) => p.enabled) &&
                  pointer.data?.state !== "ours")
              }
              onClick={() =>
                setConfirm(
                  pointer.data?.state === "ours" ? "disconnect" : "connect",
                )
              }
            >
              {text(pointer.data?.state === "ours" ? "disconnect" : "connect")}
            </Button>
            <p className="text-xs text-muted-foreground">{text("restart")}</p>
          </section>
          {confirm && (
            <section
              role="alertdialog"
              aria-label={text("confirmTitle")}
              className="space-y-3 rounded-xl border border-primary p-4"
            >
              <h3 className="font-semibold">{text("confirmTitle")}</h3>
              <p className="text-sm">
                {text(
                  confirm.startsWith("delete:")
                    ? "deleteHelp"
                    : confirm === "connect"
                      ? "connectHelp"
                      : "disconnectHelp",
                )}
              </p>
              <div className="flex gap-2">
                <Button
                  disabled={busy}
                  onClick={() =>
                    void action(
                      async () => {
                        if (confirm === "connect") await routerApi.enable();
                        else if (confirm === "disconnect")
                          await routerApi.disconnect();
                        else {
                          const id = confirm.slice(7);
                          if (mode.data === `manual:${id}`)
                            await routerApi.setMode("auto");
                          await routerApi.remove(id);
                        }
                      },
                      confirm.startsWith("delete:")
                        ? text("saved")
                        : text("restart"),
                    )
                  }
                >
                  {text("confirm")}
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => setConfirm(null)}
                >
                  {text("cancel")}
                </Button>
              </div>
            </section>
          )}
          <section className="space-y-3 rounded-xl border p-4">
            <div className="flex items-center justify-between gap-3">
              <h3 className="font-semibold">{text("usage")}</h3>
              <select
                aria-label={text("period")}
                className={`${selectClass} w-auto`}
                value={days}
                onChange={(e) => setDays(Number(e.target.value))}
              >
                {[1, 7, 30].map((n) => (
                  <option value={n} key={n}>
                    {n} {text("days")}
                  </option>
                ))}
              </select>
              <Button
                variant="ghost"
                disabled={busy}
                onClick={() =>
                  void client.invalidateQueries({ queryKey: ["router"] })
                }
              >
                {text("refresh")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {text("lowerBound")}
            </p>
            {usage.data?.length ? (
              <div className="overflow-x-auto">
                <table className="w-full text-left text-sm">
                  <thead>
                    <tr>
                      {[
                        "providers",
                        "attempts",
                        "failures",
                        "input",
                        "output",
                      ].map((key) => (
                        <th key={key} className="p-2">
                          {text(key)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {usage.data.map((row) => (
                      <tr key={row.providerId}>
                        <td className="p-2">
                          {providers.data?.find((p) => p.id === row.providerId)
                            ?.displayName ?? row.providerId}
                        </td>
                        {[
                          row.attempts,
                          row.failures,
                          row.inputTokens,
                          row.outputTokens,
                        ].map((n, i) => (
                          <td key={i} className="p-2 tabular-nums">
                            {n.toLocaleString()}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">{text("noUsage")}</p>
            )}
          </section>
        </>
      )}
    </div>
  );
}
