// Pricing table fetch / refresh + tier preference.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type { PriceTableDto, PriceTierId } from "../types";

export const pricingApi = {
  // Pricing — API-equivalent cost display for subscription users.
  pricingGet: () => invoke<PriceTableDto>("pricing_get"),
  pricingRefresh: () => invoke<PriceTableDto>("pricing_refresh"),
  // Pricing tier — which platform the user is billed through. Drives
  // the cost-report label and rate multiplier. Persisted to
  // preferences.json on every set.
  pricingTierGet: () => invoke<PriceTierId>("pricing_tier_get"),
  pricingTierSet: (tier: PriceTierId) =>
    invoke<void>("pricing_tier_set", { tier }),
};
