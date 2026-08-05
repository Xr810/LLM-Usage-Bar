/** Locale-aware "5 minutes ago" for a past millisecond timestamp. */
export function relativeTimeAgo(timestampMs: number, locale: string): string {
  const seconds = Math.max(0, Math.round((Date.now() - timestampMs) / 1000));
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const [amount, unit]: [number, Intl.RelativeTimeFormatUnit] =
    seconds < 60
      ? [seconds, "second"]
      : seconds < 3_600
        ? [Math.floor(seconds / 60), "minute"]
        : seconds < 86_400
          ? [Math.floor(seconds / 3_600), "hour"]
          : [Math.floor(seconds / 86_400), "day"];
  return format.format(-amount, unit);
}
