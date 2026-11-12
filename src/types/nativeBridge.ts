import type {
  ProviderMonitoringDashboardView,
  ProviderUsageView,
  QuotaFetchState,
  UsageProviderView,
} from "./usageDashboard";

export type NativeProviderSummaryV1 = Pick<
  UsageProviderView,
  | "id"
  | "name"
  | "billingKind"
  | "productGroupId"
  | "enabled"
  | "systemPresetKey"
>;

export interface NativeQuotaFetchStateV1 extends Omit<
  QuotaFetchState,
  "lastError"
> {
  /** Stable bridge error code only; never an upstream response or credential detail. */
  lastError: "quota_refresh_failed" | null;
}

export interface NativeProviderUsageV1 extends Omit<
  ProviderUsageView,
  "provider" | "quotaFetchState"
> {
  provider: NativeProviderSummaryV1;
  quotaFetchState: NativeQuotaFetchStateV1 | null;
}

export interface NativeProviderDashboardV1 extends Omit<
  ProviderMonitoringDashboardView,
  "providers"
> {
  providers: NativeProviderUsageV1[];
}
