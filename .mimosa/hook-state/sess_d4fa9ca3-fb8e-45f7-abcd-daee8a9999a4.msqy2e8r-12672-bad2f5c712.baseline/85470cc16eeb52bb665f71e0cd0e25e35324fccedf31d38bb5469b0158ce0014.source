import type {
  ProviderApiKeyView,
  UsageProviderView,
} from "@/types/usageDashboard";

/** The model catalogue is a property of the Provider, but fetching it needs a
    working credential. Any configured key does; prefer the first so the choice
    is stable between renders rather than jumping as keys are added. */
export function firstConfiguredKey(
  provider: UsageProviderView,
): ProviderApiKeyView | null {
  return (
    provider.apiKeys.find((key) => key.credentialStatus === "configured") ??
    null
  );
}
