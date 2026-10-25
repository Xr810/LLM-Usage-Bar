/** Public price slot. Beta has no published amount. Do not invent one. */
export type PricingPhase = "beta" | "published";

export interface PublicPricing {
  phase: PricingPhase;
  trialDays: number;
  monthlyUsd: string | null;
  yearlyUsd: string | null;
}

export const pricing: PublicPricing = {
  phase: "beta",
  trialDays: 7,
  monthlyUsd: null,
  yearlyUsd: null,
};
