/** Authorization code appended to dashboard `warnings` when a split is withheld. */
export const PRO_REQUIRED_WARNING = "PRO_REQUIRED";

export function hasProRequiredWarning(warnings: readonly string[]): boolean {
  return warnings.includes(PRO_REQUIRED_WARNING);
}

/**
 * Drop the raw authorization code. The empty state explains a withheld
 * breakdown; the code itself is not a sentence to show as an alert.
 */
export function visibleUsageWarnings(warnings: readonly string[]): string[] {
  return warnings.filter((warning) => warning !== PRO_REQUIRED_WARNING);
}
