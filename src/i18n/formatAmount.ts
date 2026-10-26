/**
 * ADR FE-9: format a USD amount before it is passed as `{{amount}}`.
 * Prices stay out of the locale files.
 */
export function formatLubAmount(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
  }).format(value);
}
