/** ADR v7.20. Auto-deny waits this many seconds unless a setting overrides it. */
export const AUTO_DENY_SECONDS_MIN = 15;
export const AUTO_DENY_SECONDS_MAX = 240;
export const AUTO_DENY_SECONDS_DEFAULT = 60;

type Translate = (
  key: string,
  options?: { ns?: string; seconds?: number },
) => string;

/** Inclusive 15–240. Missing or non-finite values stay at the default. */
export function clampAutoDenySeconds(value: number | null | undefined): number {
  if (value == null || !Number.isFinite(value))
    return AUTO_DENY_SECONDS_DEFAULT;
  const rounded = Math.round(value);
  if (rounded < AUTO_DENY_SECONDS_MIN) return AUTO_DENY_SECONDS_MIN;
  if (rounded > AUTO_DENY_SECONDS_MAX) return AUTO_DENY_SECONDS_MAX;
  return rounded;
}

/** Call site for `lub.hardstop.no_limit_on_allowing_each`. */
export function hardstopAllowEachNote(
  t: Translate,
  seconds: number = AUTO_DENY_SECONDS_DEFAULT,
): string {
  return t("lub.hardstop.no_limit_on_allowing_each", {
    ns: "pro",
    seconds: clampAutoDenySeconds(seconds),
  });
}

/** Call site for `lub.identity.auto_denies_after_s_undo`. */
export function identityAutoDenyNote(
  t: Translate,
  seconds: number = AUTO_DENY_SECONDS_DEFAULT,
): string {
  return t("lub.identity.auto_denies_after_s_undo", {
    ns: "pro",
    seconds: clampAutoDenySeconds(seconds),
  });
}
