/** C3 / C22 shell. Detected tools are a fixture. Nothing is written. */

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
}

const NOTE_KEYS: Record<string, string> = {
  env_overrides_plan: "lub.onboard.code_acme_web_env_sets",
};

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
    },
  ];
}

export function setClaudeSubscriptionProxy(input: {
  enabled: boolean;
  termsAckAt?: number | null;
}):
  | { ok: true }
  | { ok: false; error: { code: "TERMS_ACK_REQUIRED"; message: string } } {
  if (input.enabled && input.termsAckAt == null) {
    return {
      ok: false,
      error: {
        code: "TERMS_ACK_REQUIRED",
        message: "Terms acknowledgement required",
      },
    };
  }
  return { ok: true };
}
