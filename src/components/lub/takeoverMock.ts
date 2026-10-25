/** C3 / C22 shell. Detected tools are a fixture. Nothing is written or persisted. */

export const CLAUDE_PROXY_TERMS_VERSION = "r16-2026-10-25";

export type ProxyPlatform = "mac" | "windows" | "linux";

export interface DetectedTool {
  tool: string;
  name: string;
  nameEn: string;
  installed: boolean;
  authMode: "api_key" | "subscription" | "unknown" | "n_a";
  proxied: boolean;
  enforceable: boolean;
  noteCodes: string[];
  detailKey: "api_key" | null;
  /** Codex subscription is detect-only in batch 1. No checkbox and no switch. */
  control: "checkbox" | "none";
}

export interface ClaudeProxyCall {
  enabled: boolean;
  termsAckAt?: number | null;
  termsVersion?: string | null;
  platform: ProxyPlatform;
}

type ProxyError =
  | { code: "TERMS_ACK_REQUIRED"; message: string }
  | {
      code: "INVALID_ARGUMENT";
      message: string;
      details: { field: "termsAckAt" | "termsVersion" };
    }
  | {
      code: "FEATURE_UNAVAILABLE";
      message: string;
      details: { feature: "claude_subscription_proxy"; reason: "platform" };
    };

const NOTE_KEYS: Record<string, string> = {
  env_overrides_plan: "lub.onboard.code_acme_web_env_sets",
  codex_websocket: "lub.onboard.code_codex_websocket",
};

let lastProxyCall: ClaudeProxyCall | null = null;

export function lastClaudeSubscriptionProxyCall(): ClaudeProxyCall | null {
  return lastProxyCall;
}

export function noteCopyKey(code: string): string | null {
  return NOTE_KEYS[code] ?? null;
}

export function detectTools(): DetectedTool[] {
  return [
    {
      tool: "claude-code",
      name: "Claude Code",
      nameEn: "Claude Code",
      installed: true,
      authMode: "api_key",
      proxied: false,
      enforceable: true,
      noteCodes: [],
      detailKey: "api_key",
      control: "checkbox",
    },
    {
      tool: "codex",
      name: "Codex",
      nameEn: "Codex",
      installed: true,
      authMode: "api_key",
      proxied: false,
      enforceable: true,
      noteCodes: [],
      detailKey: "api_key",
      control: "checkbox",
    },
    {
      tool: "codex-subscription",
      name: "Codex 订阅",
      nameEn: "Codex subscription",
      installed: true,
      authMode: "subscription",
      proxied: false,
      enforceable: false,
      noteCodes: ["codex_websocket"],
      detailKey: null,
      control: "none",
    },
    {
      tool: "claude-plans",
      name: "Claude Max、ChatGPT Plus",
      nameEn: "Claude Max, ChatGPT Plus",
      installed: true,
      authMode: "subscription",
      proxied: false,
      enforceable: false,
      noteCodes: ["env_overrides_plan"],
      detailKey: null,
      control: "checkbox",
    },
    {
      tool: "cursor",
      name: "Cursor",
      nameEn: "Cursor",
      installed: true,
      authMode: "n_a",
      proxied: false,
      enforceable: false,
      noteCodes: [],
      detailKey: null,
      control: "checkbox",
    },
  ];
}

export function setClaudeSubscriptionProxy(input: {
  enabled: boolean;
  termsAckAt?: number | null;
  termsVersion?: string | null;
  platform?: ProxyPlatform;
}): { ok: true } | { ok: false; error: ProxyError } {
  const platform = input.platform ?? "mac";
  lastProxyCall = {
    enabled: input.enabled,
    platform,
    ...(input.termsAckAt !== undefined ? { termsAckAt: input.termsAckAt } : {}),
    ...(input.termsVersion !== undefined
      ? { termsVersion: input.termsVersion }
      : {}),
  };

  // Batch 1 writes are macOS only. Windows (and any other platform) returns
  // FEATURE_UNAVAILABLE. Closing the proxy is a write too.
  if (platform !== "mac") {
    return {
      ok: false,
      error: {
        code: "FEATURE_UNAVAILABLE",
        message: "Feature unavailable on this platform",
        details: { feature: "claude_subscription_proxy", reason: "platform" },
      },
    };
  }

  if (!input.enabled) return { ok: true };

  if (input.termsAckAt == null) {
    return {
      ok: false,
      error: {
        code: "TERMS_ACK_REQUIRED",
        message: "Terms acknowledgement required",
      },
    };
  }

  if (input.termsAckAt === 0 || input.termsAckAt > Date.now()) {
    return {
      ok: false,
      error: {
        code: "INVALID_ARGUMENT",
        message: "termsAckAt must be a past timestamp",
        details: { field: "termsAckAt" },
      },
    };
  }

  if (!input.termsVersion) {
    return {
      ok: false,
      error: {
        code: "INVALID_ARGUMENT",
        message: "termsVersion is required",
        details: { field: "termsVersion" },
      },
    };
  }

  return { ok: true };
}
