import { toast } from "sonner";
import i18n from "@/i18n";

const LUB_ERROR_KEYS = {
  FEATURE_UNAVAILABLE: "lub.errors.feature_unavailable",
} as const;

/** Copy key for a command error this UI knows how to explain. */
export function lubCommandErrorKey(code: string): string | null {
  if (code === "FEATURE_UNAVAILABLE") return LUB_ERROR_KEYS.FEATURE_UNAVAILABLE;
  return null;
}

/**
 * Defensive toast when a command still returns FEATURE_UNAVAILABLE.
 * Returns true when the code was handled. Callers must not retry.
 */
export function presentLubCommandError(code: string): boolean {
  const key = lubCommandErrorKey(code);
  if (key == null) return false;
  toast.error(i18n.t(key));
  return true;
}
