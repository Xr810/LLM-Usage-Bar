/** The credential and key-usage commands reject with a bare code string. Each
    one names a different thing the user has to do about it, so they get their
    own sentence instead of a catch-all that says only "something failed". */
const CREDENTIAL_ERRORS: Record<string, { key: string; fallback: string }> = {
  credential_conflict: {
    key: "usageDashboard.apiKeyConflict",
    fallback:
      "That key is already the saved one, or another Provider is using it. Nothing to update.",
  },
  credential_required: {
    key: "usageDashboard.apiKeyRejected",
    fallback: "That key is not usable — printable ASCII characters only.",
  },
  credential_unavailable: {
    key: "usageDashboard.apiKeyStoreUnavailable",
    fallback:
      "The system keychain could not be reached, so nothing was saved. Unlock it and try again.",
  },
  unsupported_auth: {
    key: "usageDashboard.apiKeyUnsupported",
    fallback: "This Provider does not take an API key.",
  },
  authentication_failed: {
    key: "usageDashboard.apiKeyAuthenticationFailed",
    fallback: "The Provider rejected this key. Check that it is still valid.",
  },
  upstream_rejected: {
    key: "usageDashboard.apiKeyUpstreamRejected",
    fallback: "The Provider refused the request. Try again in a moment.",
  },
  connection_failed: {
    key: "usageDashboard.apiKeyConnectionFailed",
    fallback: "Could not reach the Provider.",
  },
  invalid_response: {
    key: "usageDashboard.apiKeyInvalidResponse",
    fallback: "The Provider returned something this app could not read.",
  },
};

const FALLBACK = {
  key: "usageDashboard.providerActionFailed",
  fallback: "Unable to update this Provider.",
};

/** Backend errors arrive as bare codes; anything unmapped keeps the old wording. */
export function credentialErrorCopy(errorCode: string | null): {
  key: string;
  fallback: string;
} | null {
  if (!errorCode) return null;
  return CREDENTIAL_ERRORS[errorCode] ?? FALLBACK;
}

export function credentialErrorCodeOf(cause: unknown): string {
  const raw = cause instanceof Error ? cause.message : String(cause);
  return raw.trim();
}
