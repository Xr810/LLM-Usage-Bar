import { invoke } from "@tauri-apps/api/core";

export interface RouterProvider {
  id: string;
  displayName: string;
  baseUrl: string;
  wireApi: "responses" | "chat_completions";
  priority: number;
  enabled: boolean;
  authKind: "bearer_key" | "chatgpt_oauth" | "none";
  credentialKeyId: string | null;
}
export interface ModelRoute {
  logicalModel: string;
  upstreamModel: string;
}
export interface RouterPointer {
  state: "ours" | "not_ours" | "unreadable";
  current: string | null;
}
export interface RouterUsage {
  providerId: string;
  attempts: number;
  failures: number;
  inputTokens: number;
  outputTokens: number;
}
export const routerApi = {
  list: () => invoke<RouterProvider[]>("list_router_providers"),
  save: (input: RouterProvider) =>
    invoke<void>("upsert_router_provider", { input }),
  remove: (id: string) => invoke<void>("delete_router_provider", { id }),
  routes: (providerId: string) =>
    invoke<ModelRoute[]>("list_router_model_routes", { providerId }),
  saveRoutes: (providerId: string, routes: ModelRoute[]) =>
    invoke<void>("set_model_routes", { providerId, routes }),
  mode: () => invoke<string>("get_router_mode"),
  setMode: (mode: string) => invoke<void>("set_router_mode", { mode }),
  pointer: () => invoke<RouterPointer>("inspect_router_pointer"),
  enable: () => invoke<void>("enable_router_pointer"),
  disconnect: () => invoke<void>("disconnect_router_pointer"),
  usage: (days: number) => {
    const endAt = Math.floor(Date.now() / 1000) + 1;
    return invoke<RouterUsage[]>("recent_router_attempts", {
      startAt: endAt - days * 86400,
      endAt,
    });
  },
};
